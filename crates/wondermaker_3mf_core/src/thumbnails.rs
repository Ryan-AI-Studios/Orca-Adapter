//! Extract plate preview images embedded in Bambu/Orca project 3MF packages.

use std::io::{Read, Seek};

use base64::{Engine as _, engine::general_purpose::STANDARD as B64};
use camino::Utf8Path;
use zip::ZipArchive;

use crate::error::{Error, Result};
use crate::paths::normalize_zip_path;
use crate::zip_util::{list_entries, open_archive, read_member_bytes};

/// One plate thumbnail extracted from a project 3MF.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PlateThumbnail {
    /// 1-based plate index when known (from `plate_N.png` naming).
    pub plate_index: u32,
    /// ZIP member path (forward-slash).
    pub member_name: String,
    /// MIME type, e.g. `image/png`.
    pub mime: String,
    /// `data:{mime};base64,...` URI for WebView `<img src>`.
    pub data_url: String,
}

/// Extract plate preview images from a project `.3mf` on disk.
///
/// Looks for common Bambu/Orca names under `Metadata/`:
/// - `plate_{N}.png` (preferred)
/// - `plate_{N}_small.png`
/// - `plate_no_light_{N}.png`
/// - `top_{N}.png` / `pick_{N}.png` as last resorts
///
/// `max_plates` limits how many plate indices are probed (1..=max). Use the
/// analysis `plate_count` when known; pass 0 to auto-detect from entry names.
pub fn extract_plate_thumbnails(path: &Utf8Path, max_plates: u32) -> Result<Vec<PlateThumbnail>> {
    let mut archive = open_archive(path)?;
    extract_plate_thumbnails_archive(&mut archive, max_plates)
}

/// Extract plate thumbnails from an open archive.
pub fn extract_plate_thumbnails_archive<R: Read + Seek>(
    archive: &mut ZipArchive<R>,
    max_plates: u32,
) -> Result<Vec<PlateThumbnail>> {
    let entries = list_entries(archive)?;
    let entry_set: std::collections::HashSet<&str> =
        entries.iter().map(String::as_str).collect();

    let mut indices: Vec<u32> = if max_plates > 0 {
        (1..=max_plates).collect()
    } else {
        // Infer plate numbers from existing Metadata plate_* files.
        let mut found = Vec::new();
        for e in &entries {
            if let Some(n) = parse_plate_index_from_member(e) {
                found.push(n);
            }
        }
        found.sort_unstable();
        found.dedup();
        if found.is_empty() {
            found.push(1);
        }
        found
    };

    // Cap probe range for safety.
    indices.retain(|&i| i >= 1 && i <= 32);

    let mut out = Vec::new();
    for n in indices {
        let candidates = [
            format!("Metadata/plate_{n}.png"),
            format!("Metadata/plate_{n}_small.png"),
            format!("Metadata/plate_no_light_{n}.png"),
            format!("Metadata/top_{n}.png"),
            format!("Metadata/pick_{n}.png"),
            format!("Metadata/top_plate_{n}.png"),
            format!("Metadata/pick_plate_{n}.png"),
        ];
        let mut chosen: Option<String> = None;
        for c in &candidates {
            let norm = normalize_zip_path(c);
            if entry_set.contains(norm.as_str()) {
                chosen = Some(norm);
                break;
            }
            // Case-insensitive fallback
            if let Some(e) = entries
                .iter()
                .find(|e| e.eq_ignore_ascii_case(&norm))
            {
                chosen = Some(e.clone());
                break;
            }
        }
        let Some(member) = chosen else {
            continue;
        };
        let bytes = match read_member_bytes(archive, &member) {
            Ok(b) => b,
            Err(Error::MissingMember(_)) => continue,
            Err(e) => return Err(e),
        };
        if bytes.is_empty() {
            continue;
        }
        let mime = mime_for_member(&member);
        let data_url = format!("data:{mime};base64,{}", B64.encode(&bytes));
        out.push(PlateThumbnail {
            plate_index: n,
            member_name: member,
            mime: mime.to_string(),
            data_url,
        });
    }
    Ok(out)
}

fn mime_for_member(name: &str) -> &'static str {
    let lower = name.to_ascii_lowercase();
    if lower.ends_with(".png") {
        "image/png"
    } else if lower.ends_with(".jpg") || lower.ends_with(".jpeg") {
        "image/jpeg"
    } else if lower.ends_with(".webp") {
        "image/webp"
    } else {
        "application/octet-stream"
    }
}

/// Parse plate index from names like `Metadata/plate_2.png`, `plate_no_light_1.png`.
fn parse_plate_index_from_member(name: &str) -> Option<u32> {
    let base = name.rsplit('/').next()?.to_ascii_lowercase();
    // plate_N.png / plate_N_small.png
    if let Some(rest) = base.strip_prefix("plate_") {
        let num: String = rest.chars().take_while(|c| c.is_ascii_digit()).collect();
        if !num.is_empty() {
            return num.parse().ok();
        }
    }
    for prefix in ["plate_no_light_", "top_plate_", "pick_plate_", "top_", "pick_"] {
        if let Some(rest) = base.strip_prefix(prefix) {
            let num: String = rest.chars().take_while(|c| c.is_ascii_digit()).collect();
            if !num.is_empty() {
                return num.parse().ok();
            }
        }
    }
    None
}

#[cfg(test)]
#[allow(non_snake_case)]
mod tests {
    use super::*;
    use std::io::Cursor;
    use zip::ZipWriter;
    use zip::write::SimpleFileOptions;

    fn zip_with_png(name: &str, png_bytes: &[u8]) -> Vec<u8> {
        let mut buf = Cursor::new(Vec::new());
        {
            let mut z = ZipWriter::new(&mut buf);
            let opt = SimpleFileOptions::default();
            z.start_file(name, opt).unwrap();
            use std::io::Write;
            z.write_all(png_bytes).unwrap();
            z.finish().unwrap();
        }
        buf.into_inner()
    }

    // Minimal 1x1 PNG
    const TINY_PNG: &[u8] = &[
        0x89, 0x50, 0x4e, 0x47, 0x0d, 0x0a, 0x1a, 0x0a, 0x00, 0x00, 0x00, 0x0d, 0x49, 0x48, 0x44,
        0x52, 0x00, 0x00, 0x00, 0x01, 0x00, 0x00, 0x00, 0x01, 0x08, 0x02, 0x00, 0x00, 0x00, 0x90,
        0x77, 0x53, 0xde, 0x00, 0x00, 0x00, 0x0c, 0x49, 0x44, 0x41, 0x54, 0x08, 0xd7, 0x63, 0xf8,
        0xcf, 0xc0, 0x00, 0x00, 0x00, 0x03, 0x00, 0x01, 0x00, 0x05, 0xfe, 0xd4, 0xef, 0x00, 0x00,
        0x00, 0x00, 0x49, 0x45, 0x4e, 0x44, 0xae, 0x42, 0x60, 0x82,
    ];

    #[test]
    fn extract_plate_thumbnails__plate_1_png__returns_data_url() {
        let bytes = zip_with_png("Metadata/plate_1.png", TINY_PNG);
        let mut archive = ZipArchive::new(Cursor::new(bytes)).unwrap();
        let thumbs = extract_plate_thumbnails_archive(&mut archive, 2).expect("extract");
        assert_eq!(thumbs.len(), 1);
        assert_eq!(thumbs[0].plate_index, 1);
        assert!(thumbs[0].data_url.starts_with("data:image/png;base64,"));
        assert!(thumbs[0].data_url.len() > 40);
    }

    #[test]
    fn parse_plate_index_from_member__variants() {
        assert_eq!(
            parse_plate_index_from_member("Metadata/plate_2.png"),
            Some(2)
        );
        assert_eq!(
            parse_plate_index_from_member("Metadata/plate_1_small.png"),
            Some(1)
        );
        assert_eq!(
            parse_plate_index_from_member("Metadata/plate_no_light_3.png"),
            Some(3)
        );
        assert_eq!(parse_plate_index_from_member("Metadata/foo.png"), None);
    }
}
