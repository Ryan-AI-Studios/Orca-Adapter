//! Path helpers: disk paths use `camino`; ZIP member names are `/`-only `String`s.

use std::path::{Component, Path};

/// Well-known ZIP member paths (always forward-slash).
pub const PROJECT_SETTINGS: &str = "Metadata/project_settings.config";
pub const MODEL_SETTINGS: &str = "Metadata/model_settings.config";
pub const SLICE_INFO: &str = "Metadata/slice_info.config";
pub const CUSTOM_GCODE_PER_LAYER: &str = "Metadata/custom_gcode_per_layer.xml";
pub const FILAMENT_SEQUENCE: &str = "Metadata/filament_sequence.json";
pub const LAYER_HEIGHTS_PROFILE: &str = "Metadata/layer_heights_profile.txt";
pub const ROOT_MODEL: &str = "3D/3dmodel.model";

/// Members that must be stripped from converted output when present.
pub const STRIP_MEMBERS: &[&str] = &[
    CUSTOM_GCODE_PER_LAYER,
    FILAMENT_SEQUENCE,
    LAYER_HEIGHTS_PROFILE,
];

/// Normalize a ZIP/OPC member name to use only `/` separators and strip leading `/`.
///
/// Returns an empty string for empty input. Does not resolve `..` (archive paths are
/// treated as opaque member keys after separator normalization).
pub fn normalize_zip_path(name: &str) -> String {
    let with_fwd = name.replace('\\', "/");
    let trimmed = with_fwd.trim_start_matches('/');
    // Collapse duplicate slashes while preserving structure
    let mut out = String::with_capacity(trimmed.len());
    let mut prev_slash = false;
    for ch in trimmed.chars() {
        if ch == '/' {
            if !prev_slash {
                out.push('/');
            }
            prev_slash = true;
        } else {
            out.push(ch);
            prev_slash = false;
        }
    }
    out
}

/// True if the ZIP member should be stripped (strip list or any `*.gcode`).
pub fn should_strip_member(normalized_name: &str) -> bool {
    if STRIP_MEMBERS
        .iter()
        .any(|s| normalize_zip_path(s) == normalized_name)
    {
        return true;
    }
    // Any .gcode anywhere in the archive (plate gcode, etc.)
    Path::new(normalized_name)
        .extension()
        .is_some_and(|ext| ext.eq_ignore_ascii_case("gcode"))
}

/// True if the member is under `3D/` (geometry tree — prefer raw_copy).
pub fn is_geometry_member(normalized_name: &str) -> bool {
    normalized_name == "3D" || normalized_name.starts_with("3D/")
}

/// True if the member is a 3MF model XML under `3D/` (root or nested objects).
pub fn is_3d_model_member(normalized_name: &str) -> bool {
    is_geometry_member(normalized_name)
        && normalized_name
            .rsplit('.')
            .next()
            .is_some_and(|ext| ext.eq_ignore_ascii_case("model"))
}

/// Build default output path: insert `-zr-ultra-s` before the extension, or append it.
///
/// `foo.3mf` → `foo-zr-ultra-s.3mf`
/// `foo` → `foo-zr-ultra-s.3mf`
pub fn default_output_path(input: &camino::Utf8Path) -> camino::Utf8PathBuf {
    let parent = input.parent().unwrap_or_else(|| camino::Utf8Path::new("."));
    let stem = input.file_stem().unwrap_or("converted");
    let name = format!("{stem}-zr-ultra-s.3mf");
    parent.join(name)
}

/// Default markdown conversion report path beside the output package.
///
/// `foo-zr-ultra-s.3mf` → `foo-zr-ultra-s-conversion-report.md`
pub fn default_report_path(output: &camino::Utf8Path) -> camino::Utf8PathBuf {
    let parent = output
        .parent()
        .unwrap_or_else(|| camino::Utf8Path::new("."));
    let stem = output.file_stem().unwrap_or("converted");
    parent.join(format!("{stem}-conversion-report.md"))
}

/// Compare two disk paths for equality after best-effort canonicalization.
///
/// On failure to canonicalize either path, falls back to component-wise normalized
/// absolute comparison of the original paths.
pub fn paths_equal(a: &camino::Utf8Path, b: &camino::Utf8Path) -> bool {
    if let (Ok(ca), Ok(cb)) = (a.canonicalize_utf8(), b.canonicalize_utf8()) {
        return ca == cb;
    }
    normalize_disk_path(a) == normalize_disk_path(b)
}

fn normalize_disk_path(path: &camino::Utf8Path) -> String {
    let abs = if path.is_absolute() {
        path.to_path_buf()
    } else {
        std::env::current_dir()
            .ok()
            .and_then(|cwd| camino::Utf8PathBuf::from_path_buf(cwd).ok())
            .map(|cwd| cwd.join(path))
            .unwrap_or_else(|| path.to_path_buf())
    };
    let mut parts: Vec<String> = Vec::new();
    for c in abs.as_std_path().components() {
        match c {
            Component::Prefix(p) => {
                parts.clear();
                parts.push(p.as_os_str().to_string_lossy().to_ascii_lowercase());
            }
            Component::RootDir => {
                if parts.is_empty() {
                    parts.push(String::new());
                }
            }
            Component::CurDir => {}
            Component::ParentDir => {
                parts.pop();
            }
            Component::Normal(s) => {
                parts.push(s.to_string_lossy().to_ascii_lowercase());
            }
        }
    }
    parts.join("\\")
}

#[cfg(test)]
#[allow(non_snake_case)] // track test naming: feature__condition__expected
mod tests {
    use super::*;

    #[test]
    fn normalize_zip_path__backslash_and_leading_slash__forward_only() {
        assert_eq!(
            normalize_zip_path("3D\\Objects\\a.model"),
            "3D/Objects/a.model"
        );
        assert_eq!(
            normalize_zip_path("/Metadata/project_settings.config"),
            PROJECT_SETTINGS
        );
        assert_eq!(normalize_zip_path("a//b///c"), "a/b/c");
    }

    #[test]
    fn should_strip_member__gcode_and_list__true() {
        assert!(should_strip_member(CUSTOM_GCODE_PER_LAYER));
        assert!(should_strip_member("Metadata/plate_1.gcode"));
        assert!(should_strip_member("foo/bar.GCODE"));
        assert!(!should_strip_member(PROJECT_SETTINGS));
        assert!(!should_strip_member(MODEL_SETTINGS));
    }

    #[test]
    fn default_output_path__with_extension__suffix_before_ext() {
        let p = camino::Utf8Path::new(r"C:\models\box.3mf");
        let out = default_output_path(p);
        assert_eq!(out.as_str(), r"C:\models\box-zr-ultra-s.3mf");
    }
}
