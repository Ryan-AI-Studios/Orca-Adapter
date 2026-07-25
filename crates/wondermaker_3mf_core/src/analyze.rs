//! Analyze a Bambu/Orca project 3MF package.

use std::collections::BTreeMap;
use std::io::{Read, Seek};

use camino::Utf8Path;
use zip::ZipArchive;

use crate::error::{Error, Result};
use crate::model_settings::{count_paint_color_attrs, parse_model_settings};
use crate::paint::collect_paint_source_slots;
use crate::paths::{
    MODEL_SETTINGS, PROJECT_SETTINGS, ROOT_MODEL, is_3d_model_member, normalize_zip_path,
};
use crate::settings::{bed_size_mm, parse_project_settings, string_array_field, string_field};
use crate::zip_util::{list_entries, open_archive, read_member_bytes};

/// One filament slot (0-based index in settings arrays; display as 1-based).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FilamentInfo {
    /// 1-based slot index for UI / model_settings alignment.
    pub index_1based: u8,
    pub colour: String,
    pub type_: String,
}

/// Result of analyzing a project 3MF.
#[derive(Debug, Clone)]
pub struct Analysis {
    pub path: String,
    pub application: Option<String>,
    pub printer_model: Option<String>,
    /// Bed width × depth in mm, if inferable from `printable_area`.
    pub bed_size_mm: Option<(f64, f64)>,
    pub plate_count: u32,
    pub filaments: Vec<FilamentInfo>,
    /// Counts of 1-based extruder indices from model_settings.
    pub extruder_histogram: BTreeMap<u8, u32>,
    /// True if any `paint_color=` attribute was found in model XML.
    pub has_paint_color: bool,
    /// Number of `paint_color=` occurrences across scanned model files.
    pub paint_color_count: u32,
    /// Sorted unique 1-based source slots that must be mapped (histogram ∪ paint).
    /// Defaults to `[1]` when neither extruder nor paint slots are present.
    pub used_source_slots: Vec<u8>,
    /// Normalized ZIP member names.
    pub entries: Vec<String>,
    pub has_gcode: bool,
    pub warnings: Vec<String>,
}

/// Analyze a 3MF file on disk.
pub fn analyze(path: &Utf8Path) -> Result<Analysis> {
    let mut archive = open_archive(path)?;
    analyze_archive(&mut archive, path.as_str())
}

/// Analyze an already-open archive (used by tests with in-memory ZIPs).
pub fn analyze_archive<R: Read + Seek>(
    archive: &mut ZipArchive<R>,
    path_label: &str,
) -> Result<Analysis> {
    let entries = list_entries(archive)?;
    let has_gcode = entries.iter().any(|e| {
        std::path::Path::new(e)
            .extension()
            .is_some_and(|ext| ext.eq_ignore_ascii_case("gcode"))
    });

    // Optional for analyze: missing is a warning; present-but-corrupt is an error.
    let project_bytes = match read_member_bytes(archive, PROJECT_SETTINGS) {
        Ok(bytes) => Some(bytes),
        Err(Error::MissingMember(_)) => None,
        Err(e) => return Err(e),
    };
    let (printer_model, filaments, bed, settings_warnings) = if let Some(bytes) = project_bytes {
        let settings = parse_project_settings(&bytes)?;
        let printer = string_field(&settings, "printer_model");
        let colours = string_array_field(&settings, "filament_colour");
        let types = string_array_field(&settings, "filament_type");
        let n = colours.len().max(types.len());
        let mut filaments = Vec::with_capacity(n);
        for i in 0..n {
            let colour = colours.get(i).cloned().unwrap_or_default();
            let type_ = types.get(i).cloned().unwrap_or_default();
            let index_1based = u8::try_from(i + 1).unwrap_or(u8::MAX);
            filaments.push(FilamentInfo {
                index_1based,
                colour,
                type_,
            });
        }
        let bed = bed_size_mm(&settings);
        let mut warnings = Vec::new();
        if let Some((w, d)) = bed {
            // ZR Ultra S is 300×270 — warn when source bed is larger
            if w > 300.0 + 0.5 || d > 270.0 + 0.5 {
                warnings.push(format!(
                    "Source bed {w}×{d} mm is larger than typical ZR Ultra S 300×270 mm; objects near edges may need rearranging"
                ));
            }
        }
        (printer, filaments, bed, warnings)
    } else {
        (
            None,
            Vec::new(),
            None,
            vec!["Missing Metadata/project_settings.config".to_string()],
        )
    };

    let application = extract_application(archive)?;

    let (plate_count, extruder_histogram) = match read_member_bytes(archive, MODEL_SETTINGS) {
        Ok(bytes) => {
            let summary = parse_model_settings(&bytes)?;
            (summary.plate_count, summary.extruder_histogram)
        }
        Err(Error::MissingMember(_)) => (0, BTreeMap::new()),
        Err(e) => return Err(e),
    };

    let (paint_color_count, paint_slots) = scan_paint_colors_and_slots(archive, &entries)?;
    let has_paint_color = paint_color_count > 0;

    // Union extruder histogram keys + paint-decoded slots (same set convert validates).
    let mut used_source_slots: Vec<u8> = extruder_histogram.keys().copied().collect();
    used_source_slots.extend(paint_slots);
    used_source_slots.sort_unstable();
    used_source_slots.dedup();
    if used_source_slots.is_empty() {
        used_source_slots.push(1);
    }

    let mut warnings = settings_warnings;
    if has_gcode {
        warnings.push(
            "Archive contains .gcode members (already sliced); convert will strip them".to_string(),
        );
    }

    Ok(Analysis {
        path: path_label.to_string(),
        application,
        printer_model,
        bed_size_mm: bed,
        plate_count,
        filaments,
        extruder_histogram,
        has_paint_color,
        paint_color_count,
        used_source_slots,
        entries,
        has_gcode,
        warnings,
    })
}

fn extract_application<R: Read + Seek>(archive: &mut ZipArchive<R>) -> Result<Option<String>> {
    // Optional root model: missing → no Application metadata; present but unreadable → error.
    let bytes = match read_member_bytes(archive, ROOT_MODEL) {
        Ok(bytes) => bytes,
        Err(Error::MissingMember(_)) => return Ok(None),
        Err(e) => return Err(e),
    };
    // Lightweight scan for <metadata name="Application">...</metadata>
    let text = String::from_utf8_lossy(&bytes);
    Ok(find_metadata_value(&text, "Application"))
}

fn find_metadata_value(xml: &str, name: &str) -> Option<String> {
    // Match name="Application" then capture element text or self-closing skip
    let needle = format!("name=\"{name}\"");
    let idx = xml.find(&needle)?;
    let after = &xml[idx + needle.len()..];
    // Prefer >text</metadata>
    let gt = after.find('>')?;
    let rest = &after[gt + 1..];
    if after[..gt].contains('/') {
        return None;
    }
    let end = rest.find("</")?;
    Some(rest[..end].trim().to_string())
}

/// Scan geometry `.model` members for paint_color attrs and decode used source slots.
fn scan_paint_colors_and_slots<R: Read + Seek>(
    archive: &mut ZipArchive<R>,
    entries: &[String],
) -> Result<(u32, Vec<u8>)> {
    let mut total = 0u32;
    let mut slots: Vec<u8> = Vec::new();
    for name in entries {
        let n = normalize_zip_path(name);
        if !is_3d_model_member(&n) {
            continue;
        }
        match read_member_bytes(archive, &n) {
            Ok(bytes) => {
                total = total.saturating_add(count_paint_color_attrs(&bytes));
                slots.extend(collect_paint_source_slots(&bytes)?);
            }
            Err(Error::MissingMember(_)) => {}
            Err(e) => return Err(e),
        }
    }
    slots.sort_unstable();
    slots.dedup();
    Ok((total, slots))
}

/// Format analysis as human-readable text for CLI stdout.
pub fn format_analysis_human(a: &Analysis) -> String {
    let mut out = String::new();
    out.push_str(&format!("File: {}\n", a.path));
    out.push_str(&format!(
        "Application: {}\n",
        a.application.as_deref().unwrap_or("(unknown)")
    ));
    out.push_str(&format!(
        "Printer: {}\n",
        a.printer_model.as_deref().unwrap_or("(unknown)")
    ));
    match a.bed_size_mm {
        Some((w, d)) => out.push_str(&format!("Bed: {w}×{d} mm\n")),
        None => out.push_str("Bed: (unknown)\n"),
    }
    out.push_str(&format!("Plates: {}\n", a.plate_count));
    out.push_str(&format!(
        "Paint color: {} ({} attrs)\n",
        if a.has_paint_color { "yes" } else { "no" },
        a.paint_color_count
    ));
    out.push_str(&format!("Has G-code: {}\n", a.has_gcode));
    out.push_str("Filaments:\n");
    if a.filaments.is_empty() {
        out.push_str("  (none)\n");
    } else {
        for f in &a.filaments {
            out.push_str(&format!(
                "  #{}  {}  {}\n",
                f.index_1based, f.colour, f.type_
            ));
        }
    }
    out.push_str("Extruder histogram (1-based):\n");
    if a.extruder_histogram.is_empty() {
        out.push_str("  (none)\n");
    } else {
        for (ex, count) in &a.extruder_histogram {
            out.push_str(&format!("  extruder {ex}: {count}\n"));
        }
    }
    out.push_str("Used source slots (must map): ");
    if a.used_source_slots.is_empty() {
        out.push_str("(none)\n");
    } else {
        let list: Vec<String> = a.used_source_slots.iter().map(|s| s.to_string()).collect();
        out.push_str(&format!("{}\n", list.join(", ")));
    }
    out.push_str(&format!("ZIP entries: {}\n", a.entries.len()));
    if !a.warnings.is_empty() {
        out.push_str("Warnings:\n");
        for w in &a.warnings {
            out.push_str(&format!("  - {w}\n"));
        }
    }
    out
}
