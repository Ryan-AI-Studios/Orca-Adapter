//! Wondermaker 3MF conversion core.
//!
//! Analyze Bambu/Orca project `.3mf` packages and convert them via **S1 settings graft**
//! or **S2 template shell**:
//! - **S1:** replace `Metadata/project_settings.config` from a Wonderprint template, keep
//!   geometry and `model_settings`, patch filament colours from source, strip unsafe G-code
//!   metadata, optionally remap extruders / paint / colours via a [`SlotMap`].
//! - **S2:** geometry-only packages (no project_settings) inject `3D/**` into the template
//!   shell, merge OPC Content_Types/rels, and transplant or synthesize `model_settings`.
//!
//! # Path rules
//! - Disk paths: [`camino::Utf8Path`]
//! - ZIP/OPC member names: forward-slash [`String`] only ([`paths::normalize_zip_path`])

#![deny(unsafe_code)]

pub mod analyze;
pub mod convert;
pub mod error;
pub mod model_settings;
pub mod opc;
pub mod paint;
pub mod paths;
pub mod s2;
pub mod settings;
pub mod slot_map;
pub mod zip_util;

pub use analyze::{Analysis, FilamentInfo, analyze, analyze_archive, format_analysis_human};
pub use convert::{
    ArchiveConvertOptions, ConversionReport, ConvertOptions, ConvertStrategy, ResolvedStrategy,
    convert, convert_archives, format_report_human, format_report_markdown,
    refuse_output_equals_input, resolve_strategy,
};
pub use error::{Error, Result};
pub use opc::normalize_opc_part_name;
pub use paths::{default_output_path, default_report_path};
pub use slot_map::SlotMap;

#[cfg(test)]
mod tests_synth;
