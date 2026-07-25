//! Serializable DTOs bridging `wondermaker_3mf_core` to the frontend.

use std::collections::BTreeMap;
use std::path::Path;

use serde::{Deserialize, Serialize};
use wondermaker_3mf_core::{Analysis, ConversionReport, FilamentInfo};

/// One filament/toolhead slot for the UI.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct FilamentDto {
    #[serde(rename = "index1based")]
    pub index_1based: u8,
    pub colour: String,
    #[serde(rename = "type")]
    pub type_: String,
}

/// Analysis result returned to the frontend.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AnalysisDto {
    pub path: String,
    pub file_name: String,
    pub file_size_bytes: Option<u64>,
    pub application: Option<String>,
    pub printer_model: Option<String>,
    /// `[width_mm, depth_mm]` when known.
    pub bed_size_mm: Option<[f64; 2]>,
    pub plate_count: u32,
    pub filaments: Vec<FilamentDto>,
    /// String keys for JSON stability (`"1"`, `"2"`, …).
    pub extruder_histogram: BTreeMap<String, u32>,
    pub has_paint_color: bool,
    pub paint_color_count: u32,
    pub has_gcode: bool,
    pub warnings: Vec<String>,
    /// Sorted unique 1-based source slots that must be mapped (histogram ∪ paint).
    pub used_source_slots: Vec<u8>,
    /// Derived for UI: separate parts | painted faces | both | single | unknown.
    pub color_mode: String,
    /// Sum of extruder histogram counts (colored part instances).
    pub colored_parts: u32,
    /// Distinct colours used (filaments with histogram hits, else filament count).
    pub color_count: u32,
}

impl AnalysisDto {
    pub fn from_analysis(a: Analysis) -> Self {
        let path = a.path.clone();
        let file_name = Path::new(&path)
            .file_name()
            .map(|s| s.to_string_lossy().into_owned())
            .unwrap_or_else(|| path.clone());
        let file_size_bytes = std::fs::metadata(&path).ok().map(|m| m.len());

        let colored_parts: u32 = a.extruder_histogram.values().copied().sum();
        let hist_keys: Vec<u8> = a.extruder_histogram.keys().copied().collect();
        let color_count = if !hist_keys.is_empty() {
            hist_keys.len() as u32
        } else if !a.filaments.is_empty() {
            a.filaments.len() as u32
        } else {
            0
        };

        let has_parts = !a.extruder_histogram.is_empty();
        let color_mode = match (has_parts, a.has_paint_color) {
            (true, true) => "both",
            (true, false) => {
                if a.extruder_histogram.len() <= 1 {
                    "single"
                } else {
                    "separate parts"
                }
            }
            (false, true) => "painted faces",
            (false, false) => {
                if a.filaments.len() <= 1 {
                    "single"
                } else {
                    "unknown"
                }
            }
        }
        .to_string();

        let extruder_histogram = a
            .extruder_histogram
            .into_iter()
            .map(|(k, v)| (k.to_string(), v))
            .collect();

        let bed_size_mm = a.bed_size_mm.map(|(w, d)| [w, d]);

        Self {
            path,
            file_name,
            file_size_bytes,
            application: a.application,
            printer_model: a.printer_model,
            bed_size_mm,
            plate_count: a.plate_count,
            filaments: a.filaments.into_iter().map(FilamentDto::from).collect(),
            extruder_histogram,
            has_paint_color: a.has_paint_color,
            paint_color_count: a.paint_color_count,
            has_gcode: a.has_gcode,
            warnings: a.warnings,
            used_source_slots: a.used_source_slots,
            color_mode,
            colored_parts,
            color_count,
        }
    }
}

impl From<FilamentInfo> for FilamentDto {
    fn from(f: FilamentInfo) -> Self {
        Self {
            index_1based: f.index_1based,
            colour: f.colour,
            type_: f.type_,
        }
    }
}

/// Convert options from the frontend.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ConvertDto {
    pub source: String,
    pub template: String,
    pub output: String,
    /// CLI-style map `1=2,2=1,…`. Empty or omit → identity (with used-slot validation done in UI).
    #[serde(default)]
    pub slot_map: String,
    #[serde(default = "default_true")]
    pub copy_filament_type: bool,
    #[serde(default = "default_true")]
    pub write_report: bool,
    pub report_path: Option<String>,
    #[serde(default)]
    pub strict_bed: bool,
    /// `auto` | `s1` | `s2`
    #[serde(default = "default_auto")]
    pub strategy: String,
}

fn default_true() -> bool {
    true
}

fn default_auto() -> String {
    "auto".to_string()
}

/// Conversion report for the frontend.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ConversionReportDto {
    pub source: String,
    pub template: String,
    pub output: String,
    pub strategy: String,
    pub source_printer: Option<String>,
    pub output_printer: Option<String>,
    pub stripped_members: Vec<String>,
    pub colours_patched: bool,
    pub slot_map_identity: bool,
    pub slot_map_pairs: Vec<[u8; 2]>,
    pub paint_attrs_seen: u32,
    pub paint_attrs_rewritten: u32,
    pub had_gcode_stripped: bool,
    pub report_path: Option<String>,
    pub plates: Option<u32>,
    pub extruder_histogram_out: BTreeMap<String, u32>,
    pub colours_before: Vec<String>,
    pub colours_after: Vec<String>,
    pub warnings: Vec<String>,
    pub entry_count: usize,
    pub opc_reconciled: bool,
}

impl From<ConversionReport> for ConversionReportDto {
    fn from(r: ConversionReport) -> Self {
        let extruder_histogram_out = r
            .extruder_histogram_out
            .unwrap_or_default()
            .into_iter()
            .map(|(k, v)| (k.to_string(), v))
            .collect();
        Self {
            source: r.source.into_string(),
            template: r.template.into_string(),
            output: r.output.into_string(),
            strategy: r.strategy.as_str().to_string(),
            source_printer: r.source_printer,
            output_printer: r.output_printer,
            stripped_members: r.stripped_members,
            colours_patched: r.colours_patched,
            slot_map_identity: r.slot_map_identity,
            slot_map_pairs: r.slot_map_pairs.into_iter().map(|(a, b)| [a, b]).collect(),
            paint_attrs_seen: r.paint_attrs_seen,
            paint_attrs_rewritten: r.paint_attrs_rewritten,
            had_gcode_stripped: r.had_gcode_stripped,
            report_path: r.report_path.map(|p| p.into_string()),
            plates: r.plates,
            extruder_histogram_out,
            colours_before: r.colours_before,
            colours_after: r.colours_after,
            warnings: r.warnings,
            entry_count: r.entry_count,
            opc_reconciled: r.opc_reconciled,
        }
    }
}

/// Progress event payload (`convert-progress`).
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ProgressEvent {
    pub stage: String,
    pub index: u32,
    pub total: u32,
}

/// App config returned to frontend.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct AppConfigDto {
    pub template_path: Option<String>,
}

/// Default output path beside the source (`{stem}-zr-ultra-s.3mf`).
pub fn default_output_beside(source: &str) -> String {
    let p = camino::Utf8Path::new(source);
    wondermaker_3mf_core::default_output_path(p).into_string()
}

#[cfg(test)]
#[allow(non_snake_case)] // track-style test names: feature__condition__expected
mod tests {
    use super::*;
    use std::collections::BTreeMap;
    use wondermaker_3mf_core::Analysis;

    #[test]
    fn color_mode__parts_and_paint__both() {
        let a = Analysis {
            path: r"C:\tmp\a.3mf".into(),
            application: Some("Bambu Studio".into()),
            printer_model: Some("X1C".into()),
            bed_size_mm: Some((256.0, 256.0)),
            plate_count: 1,
            filaments: vec![],
            extruder_histogram: BTreeMap::from([(1, 2), (2, 1)]),
            has_paint_color: true,
            paint_color_count: 3,
            used_source_slots: vec![1, 2],
            entries: vec![],
            has_gcode: false,
            warnings: vec![],
        };
        let dto = AnalysisDto::from_analysis(a);
        assert_eq!(dto.color_mode, "both");
        assert_eq!(dto.colored_parts, 3);
        assert_eq!(dto.color_count, 2);
        assert_eq!(dto.used_source_slots, vec![1, 2]);
    }

    #[test]
    fn color_mode__paint_only__painted_faces() {
        let a = Analysis {
            path: r"C:\tmp\paint.3mf".into(),
            application: None,
            printer_model: None,
            bed_size_mm: None,
            plate_count: 0,
            filaments: vec![],
            extruder_histogram: BTreeMap::new(),
            has_paint_color: true,
            paint_color_count: 2,
            used_source_slots: vec![1, 16],
            entries: vec![],
            has_gcode: false,
            warnings: vec![],
        };
        let dto = AnalysisDto::from_analysis(a);
        assert_eq!(dto.color_mode, "painted faces");
        assert_eq!(dto.colored_parts, 0);
        assert_eq!(dto.used_source_slots, vec![1, 16]);
    }

    #[test]
    fn color_mode__single_part__single() {
        let a = Analysis {
            path: r"C:\tmp\one.3mf".into(),
            application: None,
            printer_model: Some("X1C".into()),
            bed_size_mm: None,
            plate_count: 1,
            filaments: vec![],
            extruder_histogram: BTreeMap::from([(1, 4)]),
            has_paint_color: false,
            paint_color_count: 0,
            used_source_slots: vec![1],
            entries: vec![],
            has_gcode: false,
            warnings: vec![],
        };
        let dto = AnalysisDto::from_analysis(a);
        assert_eq!(dto.color_mode, "single");
        assert_eq!(dto.color_count, 1);
    }

    #[test]
    fn default_output_beside__suffix() {
        let out = default_output_beside(r"C:\models\box.3mf");
        assert_eq!(out, r"C:\models\box-zr-ultra-s.3mf");
    }

    #[test]
    fn default_output_beside__no_extension() {
        let out = default_output_beside(r"C:\models\box");
        assert_eq!(out, r"C:\models\box-zr-ultra-s.3mf");
    }

    #[test]
    fn default_output_beside__forward_slashes() {
        let out = default_output_beside("C:/models/nested/part.3mf");
        assert!(out.ends_with("part-zr-ultra-s.3mf"));
        assert!(out.contains("nested"));
    }
}
