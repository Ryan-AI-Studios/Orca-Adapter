//! Wondermaker 3MF conversion core.
//!
//! Analyze Bambu/Orca project `.3mf` packages and convert them via **S1 settings graft**:
//! replace `Metadata/project_settings.config` from a Wonderprint template, keep geometry
//! and `model_settings`, patch filament colours from source, strip unsafe G-code metadata,
//! and optionally remap extruders / paint / colours via a [`SlotMap`].
//!
//! # Path rules
//! - Disk paths: [`camino::Utf8Path`]
//! - ZIP/OPC member names: forward-slash [`String`] only ([`paths::normalize_zip_path`])

#![deny(unsafe_code)]

pub mod analyze;
pub mod convert;
pub mod error;
pub mod model_settings;
pub mod paint;
pub mod paths;
pub mod settings;
pub mod slot_map;
pub mod zip_util;

pub use analyze::{Analysis, FilamentInfo, analyze, analyze_archive, format_analysis_human};
pub use convert::{
    ConversionReport, ConvertOptions, convert, convert_archives, format_report_human,
    format_report_markdown, refuse_output_equals_input,
};
pub use error::{Error, Result};
pub use paths::{default_output_path, default_report_path};
pub use slot_map::SlotMap;

#[cfg(test)]
mod tests_synth;
