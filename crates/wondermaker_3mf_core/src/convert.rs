//! S1 settings graft and S2 template-shell conversion with slot-map remaps and reports.

use std::collections::{BTreeMap, BTreeSet};
use std::io::{Read, Seek, Write};
use std::str::FromStr;

use camino::{Utf8Path, Utf8PathBuf};
use zip::{ZipArchive, ZipWriter};

use crate::error::{Error, Result};
use crate::model_meta::{
    application_stamp_from_candidate, ensure_application_metadata, read_application_metadata,
};
use crate::model_settings::{
    ExtruderRemapStats, collect_unknown_subtypes, parse_model_settings,
    remap_model_settings_extruders,
};
use crate::opc::{
    CONTENT_TYPES, is_content_types_member, is_rels_member, rels_parent_dir, remaining_extensions,
    strip_content_types_orphans, strip_rels_orphans,
};
use crate::paint::{PaintRemapStats, collect_paint_source_slots, remap_model_paint};
use crate::paths::{
    MODEL_SETTINGS, PROJECT_SETTINGS, ROOT_MODEL, SLICE_INFO, default_report_path,
    is_3d_model_member, normalize_zip_path, paths_equal, should_strip_member,
};
use crate::s2::{S2Options, convert_s2_archives};
use crate::settings::{
    BED_COMPARE_EPS_MM, bed_compare_message, bed_size_mm, bed_source_exceeds_template,
    graft_filament_colours, parse_project_settings, reorder_filament_colours,
    serialize_project_settings, string_array_field, string_field,
};
use crate::slot_map::SlotMap;
use crate::zip_util::{
    member_exists, open_archive, read_member_bytes, slice_info_stub, write_member,
};

/// Conversion strategy selection.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum ConvertStrategy {
    /// S1 when source has project_settings; S2 otherwise.
    #[default]
    Auto,
    /// Settings graft — requires source `Metadata/project_settings.config`.
    S1,
    /// Template shell + inject source geometry.
    S2,
}

impl ConvertStrategy {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Auto => "auto",
            Self::S1 => "s1",
            Self::S2 => "s2",
        }
    }
}

impl FromStr for ConvertStrategy {
    type Err = Error;

    fn from_str(s: &str) -> Result<Self> {
        match s.trim().to_ascii_lowercase().as_str() {
            "auto" => Ok(Self::Auto),
            "s1" => Ok(Self::S1),
            "s2" => Ok(Self::S2),
            other => Err(Error::msg(format!(
                "invalid strategy '{other}'; expected auto|s1|s2"
            ))),
        }
    }
}

/// Resolved strategy actually used for a conversion.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ResolvedStrategy {
    S1,
    S2,
}

impl ResolvedStrategy {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::S1 => "S1",
            Self::S2 => "S2",
        }
    }
}

/// Options for conversion (disk paths).
#[derive(Debug, Clone)]
pub struct ConvertOptions {
    pub source: Utf8PathBuf,
    pub template: Utf8PathBuf,
    pub output: Utf8PathBuf,
    /// Source slot → ZR toolhead map. Identity leaves extruder/paint/colours in place order.
    pub slot_map: SlotMap,
    /// When true, copy source `filament_colour` / multi onto toolheads (MakerWorld palette).
    /// **Default false:** keep **template** filament colours (your ZR loadout) and only
    /// remap geometry extruders/paint — matches the UI toolhead swatches.
    pub copy_source_colours: bool,
    /// When true, also copy `filament_type` labels from source (min-len).
    pub copy_filament_type: bool,
    /// When true, write a markdown conversion report. **Default false** (opt-in).
    pub write_report: bool,
    /// Optional explicit report path; default is `<output-stem>-conversion-report.md`.
    pub report_path: Option<Utf8PathBuf>,
    /// When true, error if source bed exceeds template bed (eps ~0.5 mm).
    pub strict_bed: bool,
    /// Conversion strategy (default Auto).
    pub strategy: ConvertStrategy,
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
            copy_source_colours: false,
            copy_filament_type: true,
            write_report: false,
            report_path: None,
            strict_bed: false,
            strategy: ConvertStrategy::Auto,
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

/// In-memory conversion options (tests / library callers without disk paths).
#[derive(Debug, Clone)]
pub struct ArchiveConvertOptions {
    pub slot_map: SlotMap,
    /// See [`ConvertOptions::copy_source_colours`].
    pub copy_source_colours: bool,
    pub copy_filament_type: bool,
    pub strict_bed: bool,
    pub strategy: ConvertStrategy,
}

impl Default for ArchiveConvertOptions {
    fn default() -> Self {
        Self {
            slot_map: SlotMap::identity(),
            copy_source_colours: false,
            copy_filament_type: true,
            strict_bed: false,
            strategy: ConvertStrategy::Auto,
        }
    }
}

impl ArchiveConvertOptions {
    pub fn with_slot_map(mut self, map: SlotMap) -> Self {
        self.slot_map = map;
        self
    }

    pub fn with_copy_source_colours(mut self, yes: bool) -> Self {
        self.copy_source_colours = yes;
        self
    }
}

/// Report produced by a successful convert.
#[derive(Debug, Clone)]
pub struct ConversionReport {
    pub source: Utf8PathBuf,
    pub template: Utf8PathBuf,
    pub output: Utf8PathBuf,
    pub strategy: ResolvedStrategy,
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
    /// True when Content_Types / rels were rewritten for orphan strip or S2 merge.
    pub opc_reconciled: bool,
}

/// Convert via strategy Auto/S1/S2 (disk paths).
pub fn convert(opts: &ConvertOptions) -> Result<ConversionReport> {
    if paths_equal(&opts.source, &opts.output) {
        return Err(Error::OutputEqualsInput(opts.output.clone()));
    }
    if paths_equal(&opts.template, &opts.output) {
        return Err(Error::msg(format!(
            "output path must differ from template path: {}",
            opts.output
        )));
    }

    let mut source_archive = open_archive(&opts.source)?;
    let mut template_archive = open_archive(&opts.template)?;

    let has_ps = member_exists(&mut source_archive, PROJECT_SETTINGS);
    let resolved = resolve_strategy(opts.strategy, has_ps)?;

    let archive_opts = ArchiveConvertOptions {
        slot_map: opts.slot_map.clone(),
        copy_source_colours: opts.copy_source_colours,
        copy_filament_type: opts.copy_filament_type,
        strict_bed: opts.strict_bed,
        strategy: match resolved {
            ResolvedStrategy::S1 => ConvertStrategy::S1,
            ResolvedStrategy::S2 => ConvertStrategy::S2,
        },
    };

    // Write to a temp buffer then to disk so S1/S2 share archive path.
    let mut out_buf = std::io::Cursor::new(Vec::new());
    let mut report = convert_archives(
        &mut source_archive,
        &mut template_archive,
        &mut out_buf,
        &archive_opts,
    )?;

    // Materialize to disk.
    if let Some(parent) = opts.output.parent()
        && !parent.as_str().is_empty()
        && !parent.exists()
    {
        std::fs::create_dir_all(parent.as_std_path()).map_err(|e| Error::io(parent, e))?;
    }
    std::fs::write(opts.output.as_std_path(), out_buf.into_inner())
        .map_err(|e| Error::io(&opts.output, e))?;

    report.source = opts.source.clone();
    report.template = opts.template.clone();
    report.output = opts.output.clone();

    let report_path = if opts.write_report {
        let path = opts
            .report_path
            .clone()
            .unwrap_or_else(|| default_report_path(&opts.output));
        write_report_file(&path, &report)?;
        Some(path)
    } else {
        None
    };
    report.report_path = report_path;

    Ok(report)
}

/// Resolve Auto/S1/S2 against whether source has project_settings.
pub fn resolve_strategy(
    requested: ConvertStrategy,
    source_has_project_settings: bool,
) -> Result<ResolvedStrategy> {
    match requested {
        ConvertStrategy::Auto => {
            if source_has_project_settings {
                Ok(ResolvedStrategy::S1)
            } else {
                Ok(ResolvedStrategy::S2)
            }
        }
        ConvertStrategy::S1 => {
            if source_has_project_settings {
                Ok(ResolvedStrategy::S1)
            } else {
                Err(Error::msg(
                    "strategy S1 requires Metadata/project_settings.config in the source package; \
                     use --strategy s2 or auto for geometry-only packages",
                ))
            }
        }
        ConvertStrategy::S2 => Ok(ResolvedStrategy::S2),
    }
}

/// Convert using in-memory archives (for unit tests and library callers).
///
/// Writes the output ZIP bytes into `output_writer`. Does not write a markdown file
/// (call [`format_report_markdown`] on the returned report if needed).
pub fn convert_archives<R1, R2, W>(
    source: &mut ZipArchive<R1>,
    template: &mut ZipArchive<R2>,
    output_writer: W,
    opts: &ArchiveConvertOptions,
) -> Result<ConversionReport>
where
    R1: Read + Seek,
    R2: Read + Seek,
    W: Write + Seek,
{
    let has_ps = member_exists(source, PROJECT_SETTINGS);
    let resolved = resolve_strategy(opts.strategy, has_ps)?;

    match resolved {
        ResolvedStrategy::S1 => convert_s1_archives(source, template, output_writer, opts),
        ResolvedStrategy::S2 => {
            let s2_opts = S2Options {
                slot_map: opts.slot_map.clone(),
                strict_bed: opts.strict_bed,
            };
            let build = convert_s2_archives(source, template, output_writer, &s2_opts)?;
            Ok(ConversionReport {
                source: Utf8PathBuf::from("memory://source"),
                template: Utf8PathBuf::from("memory://template"),
                output: Utf8PathBuf::from("memory://output"),
                strategy: ResolvedStrategy::S2,
                source_printer: build.source_printer,
                output_printer: build.output_printer,
                stripped_members: build.stripped_members,
                colours_patched: build.colours_patched,
                slot_map_identity: opts.slot_map.is_identity(),
                slot_map_pairs: opts.slot_map.pairs(),
                paint_attrs_seen: build.paint_attrs_seen,
                paint_attrs_rewritten: build.paint_attrs_rewritten,
                had_gcode_stripped: build.had_gcode_stripped,
                report_path: None,
                plates: build.plates,
                extruder_histogram_out: build.extruder_histogram_out,
                colours_before: build.colours_before,
                colours_after: build.colours_after,
                warnings: build.warnings,
                entry_count: build.entry_count,
                opc_reconciled: build.opc_reconciled,
            })
        }
    }
}

/// Application string to stamp on root (and nested) model XML so Wonderprint/Orca
/// does not warn that the 3MF version is newer than the app (e.g. 2.6.0.51 vs 2.3.0.1).
///
/// Uses the template Application only when it is already Wonderprint-safe; otherwise
/// forces [`crate::model_meta::DEFAULT_WONDERPRINT_APPLICATION`].
fn application_from_template<R: Read + Seek>(template: &mut ZipArchive<R>) -> String {
    let candidate = match read_member_bytes(template, ROOT_MODEL) {
        Ok(bytes) => read_application_metadata(&bytes),
        Err(_) => None,
    };
    application_stamp_from_candidate(candidate.as_deref())
}

/// Always ensure Application is set (rewrite existing tag or inject on root models).
fn stamp_application(bytes: &[u8], application: &str) -> Vec<u8> {
    ensure_application_metadata(bytes, application)
}

fn convert_s1_archives<R1, R2, W>(
    source: &mut ZipArchive<R1>,
    template: &mut ZipArchive<R2>,
    output_writer: W,
    opts: &ArchiveConvertOptions,
) -> Result<ConversionReport>
where
    R1: Read + Seek,
    R2: Read + Seek,
    W: Write + Seek,
{
    let application_stamp = application_from_template(template);
    let source_settings_bytes = read_member_bytes(source, PROJECT_SETTINGS)?;
    let template_settings_bytes = read_member_bytes(template, PROJECT_SETTINGS)?;
    let source_settings = parse_project_settings(&source_settings_bytes)?;
    let mut grafted = parse_project_settings(&template_settings_bytes)?;
    let source_printer = string_field(&source_settings, "printer_model");
    let colours_before = string_array_field(&source_settings, "filament_colour");
    let mut colours_patched = false;
    let mut warnings = Vec::new();

    // Default: keep **template** filament colours (ZR loadout = UI toolhead swatches).
    // Opt-in `copy_source_colours`: push MakerWorld palette onto toolheads + reorder by map.
    if opts.copy_source_colours {
        colours_patched = source_has_patchable_colours(&source_settings);
        graft_filament_colours(&mut grafted, &source_settings, opts.copy_filament_type);
        let colour_warns = reorder_filament_colours(&mut grafted, &source_settings, &opts.slot_map);
        warnings.extend(colour_warns.messages);
    } else if opts.copy_filament_type {
        // Types only — do not touch filament_colour / multi_colour.
        crate::settings::patch_filament_types_only(&mut grafted, &source_settings);
    }

    // Bed compare source vs template (C2).
    if let (Some(sb), Some(tb)) = (bed_size_mm(&source_settings), bed_size_mm(&grafted))
        && bed_source_exceeds_template(sb, tb, BED_COMPARE_EPS_MM)
    {
        let msg = bed_compare_message(sb, tb);
        if opts.strict_bed {
            return Err(Error::msg(msg));
        }
        warnings.push(msg);
    }

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
        for st in collect_unknown_subtypes(ms)? {
            warnings.push(format!(
                "Unsupported or unknown model_settings part subtype '{st}' (convert continues)"
            ));
        }
    }

    let identity = opts.slot_map.is_identity();

    let entry_names = crate::zip_util::list_entries(source)?;
    for name in &entry_names {
        if is_3d_model_member(name) {
            match read_member_bytes(source, name) {
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

    // First pass: stripped set for OPC.
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
    let mut stripped_members: Vec<String> = indices
        .iter()
        .filter(|(_, name)| should_strip_member(name))
        .map(|(_, name)| name.clone())
        .collect();
    stripped_members.sort();
    stripped_members.dedup();
    let stripped_set: BTreeSet<String> = stripped_members.iter().cloned().collect();

    // Remaining extensions after strip (for optional gcode Default drop).
    let remaining_names: Vec<String> = indices
        .iter()
        .filter(|(_, n)| !should_strip_member(n))
        .map(|(_, n)| n.clone())
        .collect();
    let rem_ext = remaining_extensions(
        remaining_names
            .iter()
            .chain(std::iter::once(&PROJECT_SETTINGS.to_string())),
    );

    let mut writer = ZipWriter::new(output_writer);
    let mut wrote_project_settings = false;
    let mut entry_count = 0usize;
    let mut paint_attrs_seen = 0u32;
    let mut paint_attrs_rewritten = 0u32;
    let mut extruder_histogram_out: Option<BTreeMap<u8, u32>> = None;
    let mut opc_reconciled = false;

    for (index, name) in indices {
        if should_strip_member(&name) {
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
                remap_model_settings_extruders(&bytes, &opts.slot_map)?;
            extruder_histogram_out = Some(stats.histogram_out);
            write_member(&mut writer, MODEL_SETTINGS, &remapped)?;
            entry_count += 1;
            continue;
        }

        // Stamp Application on root model always (suppress Orca "created by BambuStudio"
        // version dialog). Nested models rarely carry Application; rewrite is a no-op then.
        if is_3d_model_member(&name) {
            let bytes = {
                let mut file = source.by_index(index)?;
                let mut buf = Vec::new();
                file.read_to_end(&mut buf)
                    .map_err(|e| Error::io(format!("zip:{name}"), e))?;
                buf
            };
            let mut out_bytes = bytes;
            let mut wrote = false;
            if !identity {
                if let Some((remapped, stats)) = remap_model_paint(&out_bytes, &opts.slot_map)? {
                    let PaintRemapStats {
                        attrs_seen,
                        attrs_rewritten,
                        residual_warnings,
                    } = stats;
                    paint_attrs_seen += attrs_seen;
                    paint_attrs_rewritten += attrs_rewritten;
                    warnings.extend(residual_warnings);
                    out_bytes = remapped;
                    wrote = true;
                }
            }
            // Always stamp Application on the root model (Orca/Wonderprint version dialog).
            // Nested models only when they already carry the tag.
            let is_root = name == ROOT_MODEL;
            if is_root || read_application_metadata(&out_bytes).is_some() {
                let stamped = stamp_application(&out_bytes, &application_stamp);
                if stamped != out_bytes || is_root || wrote {
                    out_bytes = stamped;
                    wrote = true;
                }
            }
            if wrote {
                write_member(&mut writer, &name, &out_bytes)?;
                entry_count += 1;
                continue;
            }
            // Fall through to raw_copy only for nested models with no paint remap / no Application.
        }

        // OPC reconcile Content_Types / rels after strip.
        if is_content_types_member(&name) {
            let bytes = {
                let mut file = source.by_index(index)?;
                let mut buf = Vec::new();
                file.read_to_end(&mut buf)
                    .map_err(|e| Error::io(format!("zip:{name}"), e))?;
                buf
            };
            if !stripped_set.is_empty() {
                let cleaned = strip_content_types_orphans(&bytes, &stripped_set, Some(&rem_ext))?;
                write_member(&mut writer, CONTENT_TYPES, &cleaned)?;
                opc_reconciled = true;
            } else {
                write_member(&mut writer, CONTENT_TYPES, &bytes)?;
            }
            entry_count += 1;
            continue;
        }
        if is_rels_member(&name) {
            let bytes = {
                let mut file = source.by_index(index)?;
                let mut buf = Vec::new();
                file.read_to_end(&mut buf)
                    .map_err(|e| Error::io(format!("zip:{name}"), e))?;
                buf
            };
            if !stripped_set.is_empty() {
                let parent = rels_parent_dir(&name);
                let cleaned = strip_rels_orphans(&bytes, &stripped_set, &parent)?;
                write_member(&mut writer, &name, &cleaned)?;
                opc_reconciled = true;
            } else {
                write_member(&mut writer, &name, &bytes)?;
            }
            entry_count += 1;
            continue;
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

    let _ = template; // template only used for settings graft above

    Ok(ConversionReport {
        source: Utf8PathBuf::from("memory://source"),
        template: Utf8PathBuf::from("memory://template"),
        output: Utf8PathBuf::from("memory://output"),
        strategy: ResolvedStrategy::S1,
        source_printer,
        output_printer,
        stripped_members,
        colours_patched,
        slot_map_identity: identity,
        slot_map_pairs: opts.slot_map.pairs(),
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
        opc_reconciled,
    })
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
    out.push_str(&format!(
        "Conversion complete ({} {})\n",
        r.strategy.as_str(),
        match r.strategy {
            ResolvedStrategy::S1 => "settings graft",
            ResolvedStrategy::S2 => "template shell",
        }
    ));
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
        "  Application stamp: {}\n",
        crate::model_meta::DEFAULT_WONDERPRINT_APPLICATION
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
    if r.opc_reconciled {
        out.push_str("  OPC:      Content_Types/rels reconciled\n");
    }
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
    out.push_str(&format!("- **Strategy:** {}\n", r.strategy.as_str()));
    out.push_str(&format!("- **Entries written:** {}\n", r.entry_count));
    out.push_str(&format!("- **Colours patched:** {}\n", r.colours_patched));
    out.push_str(&format!("- **OPC reconciled:** {}\n", r.opc_reconciled));
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
