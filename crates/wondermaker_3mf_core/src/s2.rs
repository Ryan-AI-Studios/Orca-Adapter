//! S2 template-shell conversion: inject source geometry into a Wonderprint package.
//!
//! Used when source lacks `Metadata/project_settings.config` (geometry-only / non-Bambu).
//! Constraints **C1** (merge Content_Types/rels), **C4** (synthesize model_settings).

use std::collections::{BTreeMap, BTreeSet};
use std::io::{Read, Seek, Write};

use zip::{ZipArchive, ZipWriter};

use crate::error::{Error, Result};
use crate::model_settings::{
    collect_unknown_subtypes, parse_model_object_ids, parse_model_settings,
    remap_model_settings_extruders, synthesize_model_settings,
};
use crate::opc::{
    CONTENT_TYPES, PACKAGE_RELS, ensure_root_model_relationship, is_content_types_member,
    is_rels_member, merge_content_types_for_inject, merge_relationships, rels_parent_dir,
    remaining_extensions, strip_content_types_orphans, strip_rels_orphans,
};
use crate::paint::{collect_paint_source_slots, remap_model_paint};
use crate::paths::{
    MODEL_SETTINGS, PROJECT_SETTINGS, ROOT_MODEL, SLICE_INFO, is_3d_model_member,
    is_geometry_member, normalize_zip_path, should_strip_member,
};
use crate::settings::{
    BED_COMPARE_EPS_MM, bed_compare_message, bed_size_mm, bed_source_exceeds_template,
    parse_project_settings, serialize_project_settings, string_array_field, string_field,
};
use crate::slot_map::SlotMap;
use crate::zip_util::{read_member_bytes, slice_info_stub, write_member};

/// Shared runtime knobs for S2 (mirrors convert options without disk paths).
#[derive(Debug, Clone)]
pub struct S2Options {
    pub slot_map: SlotMap,
    pub strict_bed: bool,
}

/// Result of an S2 package build (before report wrap).
#[derive(Debug)]
pub struct S2Build {
    pub source_printer: Option<String>,
    pub output_printer: Option<String>,
    pub stripped_members: Vec<String>,
    pub colours_patched: bool,
    pub paint_attrs_seen: u32,
    pub paint_attrs_rewritten: u32,
    pub had_gcode_stripped: bool,
    pub plates: Option<u32>,
    pub extruder_histogram_out: Option<BTreeMap<u8, u32>>,
    pub colours_before: Vec<String>,
    pub colours_after: Vec<String>,
    pub warnings: Vec<String>,
    pub entry_count: usize,
    pub opc_reconciled: bool,
    pub model_settings_synthesized: bool,
}

/// Run S2: template shell + inject source `3D/**` + merge OPC + model_settings.
pub fn convert_s2_archives<R1, R2, W>(
    source: &mut ZipArchive<R1>,
    template: &mut ZipArchive<R2>,
    output_writer: W,
    opts: &S2Options,
) -> Result<S2Build>
where
    R1: Read + Seek,
    R2: Read + Seek,
    W: Write + Seek,
{
    let mut warnings = Vec::new();
    warnings.push(
        "geometry-only: template filaments (no source project_settings colour graft)".to_string(),
    );

    // Template project_settings required.
    let template_settings_bytes = read_member_bytes(template, PROJECT_SETTINGS).map_err(|e| {
        if matches!(e, Error::MissingMember(_)) {
            Error::msg("S2 requires Metadata/project_settings.config in the Wonderprint template")
        } else {
            e
        }
    })?;
    let template_settings = parse_project_settings(&template_settings_bytes)?;
    let output_printer = string_field(&template_settings, "printer_model");
    let colours_after = string_array_field(&template_settings, "filament_colour");
    let grafted_bytes = serialize_project_settings(&template_settings)?;

    // Source has no project_settings by definition of S2 auto-path; still try for bed/printer.
    let (source_printer, colours_before, source_bed) =
        match read_member_bytes(source, PROJECT_SETTINGS) {
            Ok(bytes) => {
                let s = parse_project_settings(&bytes)?;
                (
                    string_field(&s, "printer_model"),
                    string_array_field(&s, "filament_colour"),
                    bed_size_mm(&s),
                )
            }
            Err(Error::MissingMember(_)) => (None, Vec::new(), None),
            Err(e) => return Err(e),
        };

    if let (Some(sb), Some(tb)) = (source_bed, bed_size_mm(&template_settings))
        && bed_source_exceeds_template(sb, tb, BED_COMPARE_EPS_MM)
    {
        let msg = bed_compare_message(sb, tb);
        if opts.strict_bed {
            return Err(Error::msg(msg));
        }
        warnings.push(msg);
    }

    // Collect source geometry members to inject.
    let source_entries = list_file_entries(source)?;
    let mut injected: BTreeSet<String> = BTreeSet::new();
    let mut source_members: BTreeMap<String, Vec<u8>> = BTreeMap::new();
    for name in &source_entries {
        if is_geometry_member(name) && !name.ends_with('/') {
            let bytes = read_member_bytes(source, name)?;
            let n = normalize_zip_path(name);
            injected.insert(n.clone());
            source_members.insert(n, bytes);
        }
    }
    if injected.is_empty() {
        return Err(Error::msg(
            "S2: source package has no 3D/** geometry members to inject",
        ));
    }

    // Used slots from source model_settings + paint.
    let source_ms = match read_member_bytes(source, MODEL_SETTINGS) {
        Ok(b) => Some(b),
        Err(Error::MissingMember(_)) => None,
        Err(e) => return Err(e),
    };
    let mut used_sources: Vec<u8> = Vec::new();
    let mut plates = None;
    if let Some(ref ms) = source_ms {
        let summary = parse_model_settings(ms)?;
        plates = Some(summary.plate_count);
        used_sources.extend(summary.used_extruders());
        for st in collect_unknown_subtypes(ms)? {
            warnings.push(format!(
                "Unsupported or unknown model_settings part subtype '{st}' (convert continues)"
            ));
        }
    }
    for name in injected.iter().filter(|n| is_3d_model_member(n)) {
        if let Some(bytes) = source_members.get(name) {
            used_sources.extend(collect_paint_source_slots(bytes)?);
        }
    }
    used_sources.sort_unstable();
    used_sources.dedup();
    opts.slot_map
        .validate_used_map_to_zr(used_sources.iter().copied())?;
    for dest in opts
        .slot_map
        .many_to_one_dests(used_sources.iter().copied())
    {
        warnings.push(format!(
            "Many-to-one slot map into toolhead {dest} (first ascending source colour wins)"
        ));
    }

    // model_settings: transplant+remap or synthesize (C4).
    let identity = opts.slot_map.is_identity();
    let mut extruder_histogram_out = None;
    let mut model_settings_synthesized = false;
    let model_settings_out: Vec<u8> = if let Some(ms) = source_ms {
        if identity {
            extruder_histogram_out = parse_model_settings(&ms).ok().map(|s| s.extruder_histogram);
            ms
        } else {
            let (remapped, stats) = remap_model_settings_extruders(&ms, &opts.slot_map)?;
            extruder_histogram_out = Some(stats.histogram_out);
            remapped
        }
    } else {
        model_settings_synthesized = true;
        let root_bytes = source_members.get(ROOT_MODEL).ok_or_else(|| {
            Error::msg("S2: cannot synthesize model_settings without 3D/3dmodel.model")
        })?;
        let ids = parse_model_object_ids(root_bytes)?;
        let printable = ids.printable_ids();
        let synth = synthesize_model_settings(&printable, &opts.slot_map);
        if let Ok(summary) = parse_model_settings(&synth) {
            plates = Some(summary.plate_count);
            extruder_histogram_out = Some(summary.extruder_histogram);
        }
        warnings.push(
            "Synthesized Metadata/model_settings.config from root model object ids (source had none)"
                .to_string(),
        );
        synth
    };

    // Paint remap on injected models when non-identity.
    let mut paint_attrs_seen = 0u32;
    let mut paint_attrs_rewritten = 0u32;
    if !identity {
        let model_names: Vec<String> = injected
            .iter()
            .filter(|n| is_3d_model_member(n))
            .cloned()
            .collect();
        for name in model_names {
            if let Some(bytes) = source_members.get(&name).cloned()
                && let Some((remapped, stats)) = remap_model_paint(&bytes, &opts.slot_map)?
            {
                paint_attrs_seen += stats.attrs_seen;
                paint_attrs_rewritten += stats.attrs_rewritten;
                warnings.extend(stats.residual_warnings);
                source_members.insert(name, remapped);
            }
        }
    }

    // Template members (non-geometry, non-overridden).
    let template_entries = list_file_entries(template)?;
    let mut output_members: BTreeMap<String, Vec<u8>> = BTreeMap::new();
    let mut stripped_members: Vec<String> = Vec::new();

    for name in &template_entries {
        let n = normalize_zip_path(name);
        if is_geometry_member(&n) {
            // Replaced by injected source geometry.
            continue;
        }
        if should_strip_member(&n) {
            stripped_members.push(n);
            continue;
        }
        if n == PROJECT_SETTINGS || n == MODEL_SETTINGS || n == SLICE_INFO {
            continue; // written below
        }
        if is_content_types_member(&n) || is_rels_member(&n) {
            continue; // merged below
        }
        let bytes = read_member_bytes(template, &n)?;
        output_members.insert(n, bytes);
    }

    // Inject geometry (respect strip — rare under 3D).
    for (name, bytes) in &source_members {
        if should_strip_member(name) {
            stripped_members.push(name.clone());
            continue;
        }
        output_members.insert(name.clone(), bytes.clone());
    }

    // Also strip any leftover strip-list members that appeared only on source Metadata
    // (we don't inject Metadata from source except model_settings).

    // Source may still have strip members listed for report purposes.
    for name in &source_entries {
        let n = normalize_zip_path(name);
        if should_strip_member(&n) {
            stripped_members.push(n);
        }
    }

    stripped_members.sort();
    stripped_members.dedup();
    let stripped_set: BTreeSet<String> = stripped_members.iter().cloned().collect();

    // project_settings + model_settings + slice_info
    output_members.insert(PROJECT_SETTINGS.to_string(), grafted_bytes);
    output_members.insert(MODEL_SETTINGS.to_string(), model_settings_out);
    output_members.insert(SLICE_INFO.to_string(), slice_info_stub().to_vec());

    // Content_Types merge (C1).
    let template_ct = read_optional(template, CONTENT_TYPES)?;
    let source_ct = read_optional(source, CONTENT_TYPES)?;
    let mut ct = merge_content_types_for_inject(
        template_ct.as_deref(),
        source_ct.as_deref(),
        &injected,
        &stripped_set,
    )?;
    // Orphan strip against final remaining set.
    let remaining: BTreeSet<String> = output_members.keys().cloned().collect();
    let rem_ext = remaining_extensions(remaining.iter());
    ct = strip_content_types_orphans(&ct, &stripped_set, Some(&rem_ext))?;
    // Also drop overrides for parts not present in remaining (injected only those we have).
    // strip_content_types only drops stripped; good enough.
    output_members.insert(CONTENT_TYPES.to_string(), ct);

    // Package rels merge.
    let template_pkg_rels = read_optional(template, PACKAGE_RELS)?;
    let source_pkg_rels = read_optional(source, PACKAGE_RELS)?;
    let mut pkg_rels = merge_relationships(
        template_pkg_rels.as_deref(),
        source_pkg_rels.as_deref(),
        "",
        &injected,
        &stripped_set,
    )?;
    pkg_rels = ensure_root_model_relationship(Some(&pkg_rels), ROOT_MODEL)?;
    pkg_rels = strip_rels_orphans(&pkg_rels, &stripped_set, "")?;
    output_members.insert(PACKAGE_RELS.to_string(), pkg_rels);

    // 3D/_rels/* from source (and template if any).
    let mut rels_names: BTreeSet<String> = BTreeSet::new();
    for name in source_entries
        .iter()
        .chain(template_entries.iter())
        .map(|n| normalize_zip_path(n))
    {
        if is_rels_member(&name) && name != PACKAGE_RELS {
            rels_names.insert(name);
        }
    }
    for rels_name in rels_names {
        let parent = rels_parent_dir(&rels_name);
        let base = read_optional(template, &rels_name)?;
        let add = read_optional(source, &rels_name)?;
        // If only source has it, base is None — merge still works.
        if base.is_none() && add.is_none() {
            continue;
        }
        let mut merged = merge_relationships(
            base.as_deref(),
            add.as_deref(),
            &parent,
            &injected,
            &stripped_set,
        )?;
        merged = strip_rels_orphans(&merged, &stripped_set, &parent)?;
        output_members.insert(rels_name, merged);
    }

    // Write ZIP (sorted keys for determinism).
    let mut writer = ZipWriter::new(output_writer);
    let mut entry_count = 0usize;
    let names: Vec<String> = output_members.keys().cloned().collect();
    for name in names {
        if let Some(data) = output_members.get(&name) {
            write_member(&mut writer, &name, data)?;
            entry_count += 1;
        }
    }
    writer
        .finish()
        .map_err(|e| Error::msg(format!("failed to finish ZIP: {e}")))?;

    let had_gcode_stripped = stripped_members.iter().any(|m| {
        std::path::Path::new(m)
            .extension()
            .is_some_and(|ext| ext.eq_ignore_ascii_case("gcode"))
            || m.ends_with("custom_gcode_per_layer.xml")
    });
    if had_gcode_stripped {
        warnings.push(
            "**Must re-slice in Wonderprint-Orca** — pre-sliced G-code / custom layer G-code was stripped and is not valid for ZR Ultra-S toolheads."
                .to_string(),
        );
    }

    Ok(S2Build {
        source_printer,
        output_printer,
        stripped_members,
        colours_patched: false,
        paint_attrs_seen,
        paint_attrs_rewritten,
        had_gcode_stripped,
        plates,
        extruder_histogram_out,
        colours_before,
        colours_after,
        warnings,
        entry_count,
        opc_reconciled: true,
        model_settings_synthesized,
    })
}

fn list_file_entries<R: Read + Seek>(archive: &mut ZipArchive<R>) -> Result<Vec<String>> {
    let mut v = Vec::new();
    for i in 0..archive.len() {
        let file = archive.by_index(i)?;
        let name = normalize_zip_path(file.name());
        if file.is_dir() || name.ends_with('/') {
            continue;
        }
        v.push(name);
    }
    Ok(v)
}

fn read_optional<R: Read + Seek>(
    archive: &mut ZipArchive<R>,
    member: &str,
) -> Result<Option<Vec<u8>>> {
    match read_member_bytes(archive, member) {
        Ok(b) => Ok(Some(b)),
        Err(Error::MissingMember(_)) => Ok(None),
        Err(e) => Err(e),
    }
}
