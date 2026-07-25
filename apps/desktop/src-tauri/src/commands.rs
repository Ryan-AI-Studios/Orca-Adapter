//! Tauri commands: analyze, validate template, convert, config, open folder.

use std::str::FromStr;

use camino::{Utf8Path, Utf8PathBuf};
use tauri::{AppHandle, Emitter};
use wondermaker_3mf_core::{
    ConvertOptions, ConvertStrategy, SlotMap, analyze, convert,
    extract_plate_thumbnails as extract_plate_thumbs_core, refuse_output_equals_input,
};

use crate::config;
use crate::dto::{
    AnalysisDto, AppConfigDto, ConversionReportDto, ConvertDto, PlateThumbnailDto, ProgressEvent,
    default_output_beside,
};

fn utf8_path(s: &str) -> Result<Utf8PathBuf, String> {
    let trimmed = s.trim();
    if trimmed.is_empty() {
        return Err("path is empty".into());
    }
    let path = Utf8PathBuf::from_path_buf(std::path::PathBuf::from(trimmed))
        .map_err(|_| format!("path is not valid UTF-8: {trimmed}"))?;
    // T1: command boundary requires absolute paths (dialog/drop always absolute on Windows).
    if !path.is_absolute() {
        return Err(format!("path must be absolute (T1): {path}"));
    }
    Ok(path)
}

fn require_file(path: &Utf8Path) -> Result<(), String> {
    if !path.is_file() {
        return Err(format!("file not found: {path}"));
    }
    Ok(())
}

fn require_3mf_path(path: &Utf8Path, role: &str) -> Result<(), String> {
    let name = path.file_name().unwrap_or(path.as_str());
    if !name.to_ascii_lowercase().ends_with(".3mf") {
        return Err(format!("{role} must be a .3mf path, got: {path}"));
    }
    Ok(())
}

fn err_display(e: impl std::fmt::Display) -> String {
    e.to_string()
}

/// Analyze a source or any project `.3mf` via absolute path (T1).
#[tauri::command]
pub async fn analyze_3mf(source_path: String) -> Result<AnalysisDto, String> {
    let path = utf8_path(&source_path)?;
    require_3mf_path(&path, "source")?;
    require_file(&path)?;
    let analysis = tauri::async_runtime::spawn_blocking(move || analyze(&path))
        .await
        .map_err(|e| format!("analyze task failed: {e}"))?
        .map_err(err_display)?;
    Ok(AnalysisDto::from_analysis(analysis))
}

/// Analyze + light sanity checks for a Wonderprint template.
#[tauri::command]
pub async fn validate_template(template_path: String) -> Result<AnalysisDto, String> {
    let path = utf8_path(&template_path)?;
    require_3mf_path(&path, "template")?;
    require_file(&path)?;
    let analysis = tauri::async_runtime::spawn_blocking(move || analyze(&path))
        .await
        .map_err(|e| format!("validate task failed: {e}"))?
        .map_err(err_display)?;

    // Sanity: template should ideally carry project_settings (printer_model).
    // Missing printer is a soft warning already on Analysis; we still return DTO.
    Ok(AnalysisDto::from_analysis(analysis))
}

/// Extract plate preview PNGs from a project 3MF as data URLs for the WebView.
///
/// `max_plates` should be the analysis plate count (0 = auto-detect from entry names).
#[tauri::command]
pub async fn extract_plate_thumbnails(
    source_path: String,
    max_plates: u32,
) -> Result<Vec<PlateThumbnailDto>, String> {
    let path = utf8_path(&source_path)?;
    require_3mf_path(&path, "source")?;
    require_file(&path)?;
    let thumbs = tauri::async_runtime::spawn_blocking(move || {
        extract_plate_thumbs_core(&path, max_plates)
    })
    .await
    .map_err(|e| format!("thumbnail task failed: {e}"))?
    .map_err(err_display)?;
    Ok(thumbs.into_iter().map(PlateThumbnailDto::from).collect())
}

fn parse_slot_map(spec: &str) -> Result<SlotMap, String> {
    let trimmed = spec.trim();
    if trimmed.is_empty() {
        return Ok(SlotMap::identity());
    }
    SlotMap::parse(trimmed).map_err(err_display)
}

fn emit_progress(app: &AppHandle, stage: &str, index: u32, total: u32) {
    let _ = app.emit(
        "convert-progress",
        ProgressEvent {
            stage: stage.to_string(),
            index,
            total,
        },
    );
}

/// Convert in-process on a blocking pool; emit honest progress only (T2).
///
/// UI already analyzed source/template — do not re-analyze for stage theatre.
/// Single pre-convert emit; optional post-hoc emit only if a report was written.
#[tauri::command]
pub async fn convert_3mf(app: AppHandle, opts: ConvertDto) -> Result<ConversionReportDto, String> {
    let source = utf8_path(&opts.source)?;
    let template = utf8_path(&opts.template)?;
    let output = utf8_path(&opts.output)?;

    require_3mf_path(&source, "source")?;
    require_3mf_path(&template, "template")?;
    require_3mf_path(&output, "output")?;
    require_file(&source)?;
    require_file(&template)?;

    refuse_output_equals_input(&source, &output).map_err(err_display)?;
    if wondermaker_3mf_core::paths::paths_equal(&template, &output) {
        return Err(format!(
            "output path must differ from template path: {output}"
        ));
    }
    if wondermaker_3mf_core::paths::paths_equal(&source, &template) {
        return Err("source and template paths must differ".into());
    }

    let slot_map = parse_slot_map(&opts.slot_map)?;
    let strategy = ConvertStrategy::from_str(&opts.strategy).map_err(err_display)?;
    let report_path = match opts.report_path {
        Some(p) if !p.trim().is_empty() => {
            let rp = utf8_path(&p)?;
            Some(rp)
        }
        _ => None,
    };

    let convert_opts = ConvertOptions {
        source: source.clone(),
        template: template.clone(),
        output: output.clone(),
        slot_map,
        copy_source_colours: opts.copy_source_colours,
        copy_filament_type: opts.copy_filament_type,
        write_report: opts.write_report,
        report_path,
        strict_bed: opts.strict_bed,
        strategy,
    };

    // Honest milestone only — not timer-faked, not re-analyze theatre (T2 / IR1-02).
    let app_for_progress = app.clone();
    let report = tauri::async_runtime::spawn_blocking(move || {
        emit_progress(&app_for_progress, "Converting package…", 1, 1);
        convert(&convert_opts).map_err(err_display)
    })
    .await
    .map_err(|e| format!("convert task failed: {e}"))??;

    Ok(ConversionReportDto::from(report))
}

/// True if a filesystem path already exists (file or directory). Used for overwrite confirm.
#[tauri::command]
pub fn path_exists(path: String) -> Result<bool, String> {
    let path = utf8_path(&path)?;
    Ok(path.exists())
}

/// Open the parent folder of a file path, or the directory itself.
#[tauri::command]
pub async fn open_output_folder(path: String) -> Result<(), String> {
    let path = path.trim();
    if path.is_empty() {
        return Err("path is empty".into());
    }
    let p = std::path::Path::new(path);
    let target = if p.is_dir() {
        p.to_path_buf()
    } else if let Some(parent) = p.parent() {
        if parent.as_os_str().is_empty() {
            p.to_path_buf()
        } else {
            parent.to_path_buf()
        }
    } else {
        p.to_path_buf()
    };

    if !target.exists() {
        return Err(format!("folder not found: {}", target.display()));
    }

    tauri_plugin_opener::open_path(&target, None::<&str>)
        .map_err(|e| format!("open folder failed: {e}"))
}

#[tauri::command]
pub fn get_config(app: AppHandle) -> Result<AppConfigDto, String> {
    config::get_config(&app)
}

#[tauri::command]
pub fn set_template_path(app: AppHandle, template_path: Option<String>) -> Result<(), String> {
    config::set_template_path(&app, template_path)
}

/// Suggest default output path beside the source (`{stem}-zr-ultra-s.3mf`).
#[tauri::command]
pub fn suggest_output_path(source_path: String) -> Result<String, String> {
    let path = utf8_path(&source_path)?;
    require_3mf_path(&path, "source")?;
    Ok(default_output_beside(path.as_str()))
}

#[cfg(test)]
#[allow(non_snake_case)] // track-style test names: feature__condition__expected
mod tests {
    use super::*;
    use camino::Utf8Path;

    #[test]
    fn parse_slot_map__empty__identity() {
        let m = parse_slot_map("").expect("empty ok");
        assert!(m.is_identity());
        assert!(m.pairs().is_empty());
    }

    #[test]
    fn utf8_path__relative__err() {
        let err = utf8_path("relative/foo.3mf").expect_err("relative");
        assert!(err.contains("absolute"), "{err}");
    }

    #[test]
    fn utf8_path__absolute_windows__ok() {
        let p = utf8_path(r"C:\Users\test\project.3mf").expect("abs");
        assert!(p.is_absolute());
    }

    #[test]
    fn require_3mf_path__rejects_txt() {
        let p = Utf8Path::new(r"C:\out\file.txt");
        let err = require_3mf_path(p, "output").expect_err("txt");
        assert!(err.contains(".3mf"), "{err}");
    }

    #[test]
    fn require_3mf_path__accepts_3mf() {
        let p = Utf8Path::new(r"C:\out\file.3mf");
        require_3mf_path(p, "output").expect("ok");
    }

    #[test]
    fn parse_slot_map__whitespace_only__identity() {
        let m = parse_slot_map("   \t  ").expect("whitespace ok");
        assert!(m.is_identity());
    }

    #[test]
    fn parse_slot_map__swap() {
        let m = parse_slot_map("1=2,2=1").expect("parse");
        assert_eq!(m.map_slot(1), 2);
        assert_eq!(m.map_slot(2), 1);
    }

    #[test]
    fn parse_slot_map__invalid__err() {
        assert!(parse_slot_map("not-a-map").is_err());
        assert!(parse_slot_map("1=").is_err());
        assert!(parse_slot_map("=2").is_err());
        assert!(parse_slot_map("a=b").is_err());
    }

    #[test]
    fn utf8_path__empty_err() {
        assert!(utf8_path("  ").is_err());
    }

    #[test]
    fn path_exists__missing_false() {
        let p = format!(
            r"C:\dev\Wondermaker\output\__path_exists_missing_{}.3mf",
            std::process::id()
        );
        let exists = path_exists(p).expect("path_exists ok");
        assert!(!exists);
    }

    #[test]
    fn path_exists__empty_err() {
        assert!(path_exists(String::new()).is_err());
        assert!(path_exists("   ".into()).is_err());
    }

    /// Same absolute-path + core convert path the UI commands use (fixtures optional).
    #[test]
    fn desktop_command_path__dumpling_swap_map__zr_printer() {
        let source = r"C:\Users\RyanB\Documents\3D\Fidgets\toy+story+dumpling+box.3mf";
        let template = r"C:\Users\RyanB\Desktop\WonderClean.3mf";
        if !Utf8Path::new(source).is_file() || !Utf8Path::new(template).is_file() {
            eprintln!("skip: local fixtures not present");
            return;
        }
        let source = utf8_path(source).expect("abs source");
        let template = utf8_path(template).expect("abs template");
        require_3mf_path(&source, "source").expect("src 3mf");
        require_3mf_path(&template, "template").expect("tpl 3mf");

        let a = AnalysisDto::from_analysis(analyze(&source).expect("analyze source"));
        assert!(
            a.printer_model
                .as_deref()
                .is_some_and(|p| p.contains("H2C") || p.contains("Bambu")),
            "source printer: {:?}",
            a.printer_model
        );
        assert!(!a.used_source_slots.is_empty(), "used slots");

        let out = Utf8PathBuf::from(format!(
            r"C:\dev\Wondermaker\output\dumpling-zr-desktop-cmd-{}.3mf",
            std::process::id()
        ));
        require_3mf_path(&out, "output").expect("out 3mf");
        let map = parse_slot_map("1=2,2=1,3=3,4=4").expect("map");
        let opts = ConvertOptions {
            source: source.clone(),
            template: template.clone(),
            output: out.clone(),
            slot_map: map,
            copy_source_colours: false,
            copy_filament_type: true,
            write_report: false,
            report_path: None,
            strict_bed: false,
            strategy: ConvertStrategy::Auto,
        };
        let report = convert(&opts).expect("convert");
        let dto = ConversionReportDto::from(report);
        assert_eq!(dto.strategy, "S1");
        assert!(
            dto.output_printer
                .as_deref()
                .is_some_and(|p| p.contains("ZR Ultra")),
            "out printer: {:?}",
            dto.output_printer
        );
        assert!(!dto.slot_map_identity);
        assert_eq!(dto.slot_map_pairs, vec![[1, 2], [2, 1], [3, 3], [4, 4]]);
        let out_a = analyze(&out).expect("analyze out");
        assert!(
            out_a
                .printer_model
                .as_deref()
                .is_some_and(|p| p.contains("ZR Ultra")),
            "{:?}",
            out_a.printer_model
        );
        let _ = std::fs::remove_file(out.as_std_path());
        if let Some(rp) = dto.report_path {
            let _ = std::fs::remove_file(rp);
        }
    }
}
