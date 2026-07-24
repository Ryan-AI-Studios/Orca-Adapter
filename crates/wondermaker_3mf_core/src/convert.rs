//! S1 settings graft conversion with optional slot-map remaps and markdown reports.

use std::collections::BTreeMap;
use std::fs::File;
use std::io::{Read, Seek, Write};

use camino::{Utf8Path, Utf8PathBuf};
use zip::{ZipArchive, ZipWriter};

use crate::error::{Error, Result};
use crate::model_settings::{
    ExtruderRemapStats, parse_model_settings, remap_model_settings_extruders,
};
use crate::paint::{PaintRemapStats, collect_paint_source_slots, remap_model_paint};
use crate::paths::{
    MODEL_SETTINGS, PROJECT_SETTINGS, SLICE_INFO, default_report_path, is_3d_model_member,
    normalize_zip_path, paths_equal, should_strip_member,
};
use crate::settings::{
    graft_filament_colours, parse_project_settings, reorder_filament_colours,
    serialize_project_settings, string_array_field, string_field,
};
use crate::slot_map::SlotMap;
use crate::zip_util::{
    create_writer, open_archive, read_member_bytes, slice_info_stub, write_member,
};

/// Options for S1 settings-graft conversion.
#[derive(Debug, Clone)]
pub struct ConvertOptions {
    pub source: Utf8PathBuf,
    pub template: Utf8PathBuf,
    pub output: Utf8PathBuf,
    /// Source slot → ZR toolhead map. Identity leaves extruder/paint/colours in place order.
    pub slot_map: SlotMap,
    /// When true (default), also copy `filament_type` labels from source (min-len).
    pub copy_filament_type: bool,
    /// When true (default), write a markdown conversion report.
    pub write_report: bool,
    /// Optional explicit report path; default is `<output-stem>-conversion-report.md`.
    pub report_path: Option<Utf8PathBuf>,
}

impl ConvertOptions {
    pub fn new(
        source: impl Into<Utf8PathBuf>,
        template: impl Into<Utf8PathBuf>,
        output: impl Into<Utf8PathBuf>,
    ) -> Self {
        Self {
            source: source.into(),
            template: template.into(),
            output: output.into(),
            slot_map: SlotMap::identity(),
            copy_filament_type: true,
            write_report: true,
            report_path: None,
        }
    }

    /// Resolve output path: use provided, or default `-zr-ultra-s.3mf` beside source.
    pub fn with_default_output_if_empty(mut self) -> Self {
        if self.output.as_str().is_empty() {
            self.output = crate::paths::default_output_path(&self.source);
        }
        self
    }
}

/// Report produced by a successful convert.
#[derive(Debug, Clone)]
pub struct ConversionReport {
    pub source: Utf8PathBuf,
    pub template: Utf8PathBuf,
    pub output: Utf8PathBuf,
    pub source_printer: Option<String>,
    pub output_printer: Option<String>,
    pub stripped_members: Vec<String>,
    pub colours_patched: bool,
    pub slot_map_identity: bool,
    pub slot_map_pairs: Vec<(u8, u8)>,
    pub paint_attrs_seen: u32,
    pub paint_attrs_rewritten: u32,
    pub had_gcode_stripped: bool,
    pub report_path: Option<Utf8PathBuf>,
    pub plates: Option<u32>,
    pub extruder_histogram_out: Option<BTreeMap<u8, u32>>,
    pub colours_before: Vec<String>,
    pub colours_after: Vec<String>,
    pub warnings: Vec<String>,
    pub entry_count: usize,
}

/// Convert via S1 settings graft (disk paths).
pub fn convert(opts: &ConvertOptions) -> Result<ConversionReport> {
    if paths_equal(&opts.source, &opts.output) {
        return Err(Error::OutputEqualsInput(opts.output.clone()));
    }
    // Also refuse if output equals template (would clobber donor)
    if paths_equal(&opts.template, &opts.output) {
        return Err(Error::msg(format!(
            "output path must differ from template path: {}",
            opts.output
        )));
    }

    let mut source_archive = open_archive(&opts.source)?;
    let mut template_archive = open_archive(&opts.template)?;

    let source_settings_bytes = read_member_bytes(&mut source_archive, PROJECT_SETTINGS)?;
    let template_settings_bytes = read_member_bytes(&mut template_archive, PROJECT_SETTINGS)?;

    let source_settings = parse_project_settings(&source_settings_bytes)?;
    let mut grafted = parse_project_settings(&template_settings_bytes)?;

    let source_printer = string_field(&source_settings, "printer_model");
    let colours_before = string_array_field(&source_settings, "filament_colour");
    let colours_patched = source_has_patchable_colours(&source_settings);
    graft_filament_colours(&mut grafted, &source_settings, opts.copy_filament_type);

    let mut warnings = Vec::new();
    let colour_warns = reorder_filament_colours(&mut grafted, &source_settings, &opts.slot_map);
    warnings.extend(colour_warns.messages);

    let colours_after = string_array_field(&grafted, "filament_colour");
    let output_printer = string_field(&grafted, "printer_model");
    let grafted_bytes = serialize_project_settings(&grafted)?;

    // Bed size warn from source
    if let Some((w, d)) = crate::settings::bed_size_mm(&source_settings)
        && (w > 300.0 + 0.5 || d > 270.0 + 0.5)
    {
        warnings.push(format!(
            "Source bed {w}×{d} mm exceeds typical ZR Ultra S 300×270 mm"
        ));
    }

    // Pre-scan model_settings + paint for used slots; always validate dest ∈ 1..=4
    // (including identity: slot 5→5 is out of ZR range and must error).
    let model_settings_bytes = match read_member_bytes(&mut source_archive, MODEL_SETTINGS) {
        Ok(b) => Some(b),
        Err(Error::MissingMember(_)) => None,
        Err(e) => return Err(e),
    };

    let mut plates = None;
    let mut used_sources: Vec<u8> = Vec::new();
    if let Some(ref ms) = model_settings_bytes {
        let summary = parse_model_settings(ms)?;
        plates = Some(summary.plate_count);
        used_sources.extend(summary.used_extruders());
    }

    let entries = crate::zip_util::list_entries(&mut source_archive)?;
    for name in entries {
        if is_3d_model_member(&name) {
            match read_member_bytes(&mut source_archive, &name) {
                Ok(bytes) => {
                    let slots = collect_paint_source_slots(&bytes)?;
                    used_sources.extend(slots);
                }
                Err(Error::MissingMember(_)) => {}
                Err(e) => return Err(e),
            }
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

    let mut stripped_members = Vec::new();
    let mut paint_attrs_seen = 0u32;
    let mut paint_attrs_rewritten = 0u32;
    let mut extruder_histogram_out: Option<BTreeMap<u8, u32>> = None;

    let mut writer = create_writer(&opts.output)?;

    // Process each source entry by index so we can raw_copy where possible.
    drop(source_archive);
    let source_file =
        File::open(opts.source.as_std_path()).map_err(|e| Error::io(&opts.source, e))?;
    let mut source_archive = ZipArchive::new(source_file)?;

    let mut wrote_project_settings = false;
    let mut wrote_slice_info = false;
    let mut entry_count = 0usize;

    let indices: Vec<(usize, String)> = {
        let mut v = Vec::with_capacity(source_archive.len());
        for i in 0..source_archive.len() {
            let file = source_archive.by_index(i)?;
            let name = normalize_zip_path(file.name());
            if file.is_dir() || name.ends_with('/') {
                continue;
            }
            v.push((i, name));
        }
        v
    };

    let identity = opts.slot_map.is_identity();

    for (index, name) in indices {
        if should_strip_member(&name) {
            stripped_members.push(name);
            continue;
        }

        if name == PROJECT_SETTINGS {
            write_member(&mut writer, PROJECT_SETTINGS, &grafted_bytes)?;
            wrote_project_settings = true;
            entry_count += 1;
            continue;
        }

        if name == SLICE_INFO {
            write_member(&mut writer, SLICE_INFO, slice_info_stub())?;
            wrote_slice_info = true;
            entry_count += 1;
            continue;
        }

        if name == MODEL_SETTINGS && !identity {
            let bytes = read_member_bytes(&mut source_archive, MODEL_SETTINGS)?;
            let (remapped, stats) = remap_model_settings_extruders(&bytes, &opts.slot_map)?;
            extruder_histogram_out = Some(stats.histogram_out.clone());
            write_member(&mut writer, MODEL_SETTINGS, &remapped)?;
            entry_count += 1;
            let _ = stats;
            continue;
        }

        if is_3d_model_member(&name) && !identity {
            let bytes = {
                let mut file = source_archive.by_index(index)?;
                let mut buf = Vec::new();
                file.read_to_end(&mut buf)
                    .map_err(|e| Error::io(format!("zip:{name}"), e))?;
                buf
            };
            if let Some((remapped, stats)) = remap_model_paint(&bytes, &opts.slot_map)? {
                paint_attrs_seen += stats.attrs_seen;
                paint_attrs_rewritten += stats.attrs_rewritten;
                for w in stats.residual_warnings {
                    warnings.push(w);
                }
                write_member(&mut writer, &name, &remapped)?;
                entry_count += 1;
                continue;
            }
            // No paint_color — fall through to raw_copy
        }

        copy_source_member(&mut writer, &mut source_archive, index, &name)?;
        entry_count += 1;
    }

    if !wrote_project_settings {
        write_member(&mut writer, PROJECT_SETTINGS, &grafted_bytes)?;
        entry_count += 1;
    }
    if !wrote_slice_info {
        let _ = wrote_slice_info;
    }

    writer
        .finish()
        .map_err(|e| Error::msg(format!("failed to finish ZIP: {e}")))?;

    stripped_members.sort();
    stripped_members.dedup();

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

    // If identity, still expose input extruder histogram as output when available.
    if extruder_histogram_out.is_none()
        && let Some(ref ms) = model_settings_bytes
        && let Ok(summary) = parse_model_settings(ms)
    {
        extruder_histogram_out = Some(summary.extruder_histogram);
    }

    drop(template_archive);

    let report_path = if opts.write_report {
        let path = opts
            .report_path
            .clone()
            .unwrap_or_else(|| default_report_path(&opts.output));
        Some(path)
    } else {
        None
    };

    let report = ConversionReport {
        source: opts.source.clone(),
        template: opts.template.clone(),
        output: opts.output.clone(),
        source_printer,
        output_printer,
        stripped_members,
        colours_patched,
        slot_map_identity: identity,
        slot_map_pairs: opts.slot_map.pairs(),
        paint_attrs_seen,
        paint_attrs_rewritten,
        had_gcode_stripped,
        report_path: report_path.clone(),
        plates,
        extruder_histogram_out,
        colours_before,
        colours_after,
        warnings,
        entry_count,
    };

    if let Some(ref path) = report_path {
        write_report_file(path, &report)?;
    }

    Ok(report)
}

fn write_report_file(path: &Utf8Path, report: &ConversionReport) -> Result<()> {
    if let Some(parent) = path.parent()
        && !parent.as_str().is_empty()
        && !parent.exists()
    {
        std::fs::create_dir_all(parent.as_std_path()).map_err(|e| Error::io(parent, e))?;
    }
    let md = format_report_markdown(report);
    std::fs::write(path.as_std_path(), md).map_err(|e| Error::io(path, e))?;
    Ok(())
}

/// Convert using in-memory archives (for unit tests).
///
/// Writes the output ZIP bytes into `output_writer`. Does not write a markdown file
/// (call [`format_report_markdown`] on the returned report if needed).
pub fn convert_archives<R1, R2, W>(
    source: &mut ZipArchive<R1>,
    template: &mut ZipArchive<R2>,
    output_writer: W,
    slot_map: &SlotMap,
    copy_filament_type: bool,
) -> Result<ConversionReport>
where
    R1: Read + Seek,
    R2: Read + Seek,
    W: Write + Seek,
{
    let source_settings_bytes = read_member_bytes(source, PROJECT_SETTINGS)?;
    let template_settings_bytes = read_member_bytes(template, PROJECT_SETTINGS)?;
    let source_settings = parse_project_settings(&source_settings_bytes)?;
    let mut grafted = parse_project_settings(&template_settings_bytes)?;
    let source_printer = string_field(&source_settings, "printer_model");
    let colours_before = string_array_field(&source_settings, "filament_colour");
    let colours_patched = source_has_patchable_colours(&source_settings);
    graft_filament_colours(&mut grafted, &source_settings, copy_filament_type);

    let mut warnings = Vec::new();
    let colour_warns = reorder_filament_colours(&mut grafted, &source_settings, slot_map);
    warnings.extend(colour_warns.messages);

    let colours_after = string_array_field(&grafted, "filament_colour");
    let output_printer = string_field(&grafted, "printer_model");
    let grafted_bytes = serialize_project_settings(&grafted)?;

    let model_settings_bytes = match read_member_bytes(source, MODEL_SETTINGS) {
        Ok(b) => Some(b),
        Err(Error::MissingMember(_)) => None,
        Err(e) => return Err(e),
    };

    let mut plates = None;
    let mut used_sources: Vec<u8> = Vec::new();
    if let Some(ref ms) = model_settings_bytes {
        let summary = parse_model_settings(ms)?;
        plates = Some(summary.plate_count);
        used_sources.extend(summary.used_extruders());
    }

    let identity = slot_map.is_identity();

    // Always collect paint slots and validate dest ∈ 1..=4 (identity 5→5 must error).
    let entry_names = crate::zip_util::list_entries(source)?;
    for name in entry_names {
        if is_3d_model_member(&name) {
            match read_member_bytes(source, &name) {
                Ok(bytes) => {
                    used_sources.extend(collect_paint_source_slots(&bytes)?);
                }
                Err(Error::MissingMember(_)) => {}
                Err(e) => return Err(e),
            }
        }
    }
    used_sources.sort_unstable();
    used_sources.dedup();
    slot_map.validate_used_map_to_zr(used_sources.iter().copied())?;
    for dest in slot_map.many_to_one_dests(used_sources.iter().copied()) {
        warnings.push(format!(
            "Many-to-one slot map into toolhead {dest} (first ascending source colour wins)"
        ));
    }

    let mut writer = ZipWriter::new(output_writer);
    let mut stripped_members = Vec::new();
    let mut wrote_project_settings = false;
    let mut entry_count = 0usize;
    let mut paint_attrs_seen = 0u32;
    let mut paint_attrs_rewritten = 0u32;
    let mut extruder_histogram_out: Option<BTreeMap<u8, u32>> = None;

    let indices: Vec<(usize, String)> = {
        let mut v = Vec::new();
        for i in 0..source.len() {
            let file = source.by_index(i)?;
            let name = normalize_zip_path(file.name());
            if file.is_dir() || name.ends_with('/') {
                continue;
            }
            v.push((i, name));
        }
        v
    };

    for (index, name) in indices {
        if should_strip_member(&name) {
            stripped_members.push(name);
            continue;
        }
        if name == PROJECT_SETTINGS {
            write_member(&mut writer, PROJECT_SETTINGS, &grafted_bytes)?;
            wrote_project_settings = true;
            entry_count += 1;
            continue;
        }
        if name == SLICE_INFO {
            write_member(&mut writer, SLICE_INFO, slice_info_stub())?;
            entry_count += 1;
            continue;
        }

        if name == MODEL_SETTINGS && !identity {
            let bytes = read_member_bytes(source, MODEL_SETTINGS)?;
            let (remapped, stats): (Vec<u8>, ExtruderRemapStats) =
                remap_model_settings_extruders(&bytes, slot_map)?;
            extruder_histogram_out = Some(stats.histogram_out);
            write_member(&mut writer, MODEL_SETTINGS, &remapped)?;
            entry_count += 1;
            continue;
        }

        if is_3d_model_member(&name) && !identity {
            let bytes = {
                let mut file = source.by_index(index)?;
                let mut buf = Vec::new();
                file.read_to_end(&mut buf)
                    .map_err(|e| Error::io(format!("zip:{name}"), e))?;
                buf
            };
            if let Some((remapped, stats)) = remap_model_paint(&bytes, slot_map)? {
                let PaintRemapStats {
                    attrs_seen,
                    attrs_rewritten,
                    residual_warnings,
                } = stats;
                paint_attrs_seen += attrs_seen;
                paint_attrs_rewritten += attrs_rewritten;
                warnings.extend(residual_warnings);
                write_member(&mut writer, &name, &remapped)?;
                entry_count += 1;
                continue;
            }
        }

        copy_source_member(&mut writer, source, index, &name)?;
        entry_count += 1;
    }

    if !wrote_project_settings {
        write_member(&mut writer, PROJECT_SETTINGS, &grafted_bytes)?;
        entry_count += 1;
    }

    writer
        .finish()
        .map_err(|e| Error::msg(format!("failed to finish ZIP: {e}")))?;

    stripped_members.sort();
    stripped_members.dedup();

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

    if extruder_histogram_out.is_none()
        && let Some(ref ms) = model_settings_bytes
        && let Ok(summary) = parse_model_settings(ms)
    {
        extruder_histogram_out = Some(summary.extruder_histogram);
    }

    Ok(ConversionReport {
        source: Utf8PathBuf::from("memory://source"),
        template: Utf8PathBuf::from("memory://template"),
        output: Utf8PathBuf::from("memory://output"),
        source_printer,
        output_printer,
        stripped_members,
        colours_patched,
        slot_map_identity: identity,
        slot_map_pairs: slot_map.pairs(),
        paint_attrs_seen,
        paint_attrs_rewritten,
        had_gcode_stripped,
        report_path: None,
        plates,
        extruder_histogram_out,
        colours_before,
        colours_after,
        warnings,
        entry_count,
    })
}

/// True when source has non-empty colour arrays that graft will apply.
fn source_has_patchable_colours(source_settings: &serde_json::Value) -> bool {
    !string_array_field(source_settings, "filament_colour").is_empty()
        || !string_array_field(source_settings, "filament_multi_colour").is_empty()
}

/// Copy one source ZIP member into the output writer, enforcing `/`-only names.
fn copy_source_member<R, W>(
    writer: &mut ZipWriter<W>,
    source: &mut ZipArchive<R>,
    index: usize,
    normalized_name: &str,
) -> Result<()>
where
    R: Read + Seek,
    W: Write + Seek,
{
    let file = source.by_index(index)?;
    let original_name = file.name().to_string();
    if original_name == normalized_name {
        writer.raw_copy_file(file).map_err(Error::from)?;
    } else {
        writer
            .raw_copy_file_rename(file, normalized_name)
            .map_err(Error::from)?;
    }
    Ok(())
}

/// Format conversion report for CLI stdout.
pub fn format_report_human(r: &ConversionReport) -> String {
    let mut out = String::new();
    out.push_str("Conversion complete (S1 settings graft)\n");
    out.push_str(&format!("  Source:   {}\n", r.source));
    out.push_str(&format!("  Template: {}\n", r.template));
    out.push_str(&format!("  Output:   {}\n", r.output));
    if let Some(ref rp) = r.report_path {
        out.push_str(&format!("  Report:   {rp}\n"));
    }
    out.push_str(&format!(
        "  Printer:  {} → {}\n",
        r.source_printer.as_deref().unwrap_or("?"),
        r.output_printer.as_deref().unwrap_or("?")
    ));
    out.push_str(&format!(
        "  Colours patched from source: {}\n",
        r.colours_patched
    ));
    out.push_str(&format!("  Slot map identity: {}\n", r.slot_map_identity));
    if !r.slot_map_pairs.is_empty() {
        let pairs: Vec<String> = r
            .slot_map_pairs
            .iter()
            .map(|(s, d)| format!("{s}→{d}"))
            .collect();
        out.push_str(&format!("  Slot map: {}\n", pairs.join(", ")));
    }
    if let Some(plates) = r.plates {
        out.push_str(&format!("  Plates:   {plates}\n"));
    }
    if r.paint_attrs_seen > 0 {
        out.push_str(&format!(
            "  Paint:    {} attrs seen, {} rewritten\n",
            r.paint_attrs_seen, r.paint_attrs_rewritten
        ));
    }
    out.push_str(&format!("  Entries written: {}\n", r.entry_count));
    if r.stripped_members.is_empty() {
        out.push_str("  Stripped: (none)\n");
    } else {
        out.push_str("  Stripped:\n");
        for m in &r.stripped_members {
            out.push_str(&format!("    - {m}\n"));
        }
    }
    if r.had_gcode_stripped {
        out.push_str("  **Must re-slice in Wonderprint-Orca** (G-code was stripped)\n");
    }
    if !r.warnings.is_empty() {
        out.push_str("  Warnings:\n");
        for w in &r.warnings {
            out.push_str(&format!("    - {w}\n"));
        }
    }
    out
}

/// Format a durable markdown conversion report.
pub fn format_report_markdown(r: &ConversionReport) -> String {
    let mut out = String::new();
    out.push_str("# Wondermaker Conversion Report\n\n");

    out.push_str("## Summary\n\n");
    out.push_str(&format!("- **Source:** `{}`\n", r.source));
    out.push_str(&format!("- **Template:** `{}`\n", r.template));
    out.push_str(&format!("- **Output:** `{}`\n", r.output));
    out.push_str(&format!("- **Entries written:** {}\n", r.entry_count));
    out.push_str(&format!("- **Colours patched:** {}\n", r.colours_patched));
    if let Some(plates) = r.plates {
        out.push_str(&format!("- **Plates:** {plates}\n"));
    }
    out.push('\n');

    out.push_str("## Printer\n\n");
    out.push_str(&format!(
        "- **Source printer:** {}\n",
        r.source_printer.as_deref().unwrap_or("(unknown)")
    ));
    out.push_str(&format!(
        "- **Output printer:** {}\n",
        r.output_printer.as_deref().unwrap_or("(unknown)")
    ));
    out.push('\n');

    out.push_str("## Slot map\n\n");
    if r.slot_map_identity && r.slot_map_pairs.is_empty() {
        out.push_str("Identity (no remapping).\n\n");
    } else {
        out.push_str("| Source | Destination |\n");
        out.push_str("| ---: | ---: |\n");
        if r.slot_map_pairs.is_empty() {
            out.push_str("| (identity) | (identity) |\n");
        } else {
            for (s, d) in &r.slot_map_pairs {
                out.push_str(&format!("| {s} | {d} |\n"));
            }
        }
        out.push('\n');
    }

    out.push_str("## Colours\n\n");
    out.push_str(&format!(
        "- **Before (source):** {}\n",
        format_colour_list(&r.colours_before)
    ));
    out.push_str(&format!(
        "- **After (output):** {}\n",
        format_colour_list(&r.colours_after)
    ));
    out.push('\n');

    if let Some(ref hist) = r.extruder_histogram_out {
        out.push_str("## Extruders (output histogram)\n\n");
        if hist.is_empty() {
            out.push_str("(none)\n\n");
        } else {
            out.push_str("| Toolhead | Count |\n");
            out.push_str("| ---: | ---: |\n");
            for (slot, count) in hist {
                out.push_str(&format!("| {slot} | {count} |\n"));
            }
            out.push('\n');
        }
    }

    out.push_str("## Stripped\n\n");
    if r.stripped_members.is_empty() {
        out.push_str("(none)\n\n");
    } else {
        for m in &r.stripped_members {
            out.push_str(&format!("- `{m}`\n"));
        }
        out.push('\n');
    }
    if r.had_gcode_stripped {
        out.push_str(
            "**Must re-slice in Wonderprint-Orca** — pre-sliced G-code was stripped and must not be used on ZR Ultra-S.\n\n",
        );
    }

    out.push_str("## Paint\n\n");
    out.push_str(&format!("- **Attributes seen:** {}\n", r.paint_attrs_seen));
    out.push_str(&format!(
        "- **Attributes rewritten:** {}\n",
        r.paint_attrs_rewritten
    ));
    out.push('\n');

    out.push_str("## Warnings\n\n");
    if r.warnings.is_empty() {
        out.push_str("(none)\n");
    } else {
        for w in &r.warnings {
            out.push_str(&format!("- {w}\n"));
        }
    }
    out.push('\n');
    out
}

fn format_colour_list(colours: &[String]) -> String {
    if colours.is_empty() {
        "(none)".to_string()
    } else {
        colours
            .iter()
            .enumerate()
            .map(|(i, c)| format!("{}:{}", i + 1, c))
            .collect::<Vec<_>>()
            .join(", ")
    }
}

/// Safety check used by CLI and tests.
pub fn refuse_output_equals_input(source: &Utf8Path, output: &Utf8Path) -> Result<()> {
    if paths_equal(source, output) {
        Err(Error::OutputEqualsInput(output.to_path_buf()))
    } else {
        Ok(())
    }
}
