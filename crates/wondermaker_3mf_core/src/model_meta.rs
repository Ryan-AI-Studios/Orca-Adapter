//! Lightweight rewrites of 3MF model XML metadata (Application string, etc.).

/// Default Application string used by Wonderprint-Orca 2.3.x templates.
///
/// Wonderprint / Orca parse the version **after** `BambuStudio-` (or `OrcaSlicer-`)
/// and compare it to the running app. Values like `BambuStudio-02.06.00.51` trigger
/// “The 3mf's version 2.6.0.51 is newer than Wonderprint-Orca's version 2.3.0.1…”.
/// Stamping `BambuStudio-2.3.1` keeps the file on the pre-2.3.2 Bambu-compatible path
/// (no newer-version dialog when project_settings already come from a Wonderprint template).
pub const DEFAULT_WONDERPRINT_APPLICATION: &str = "BambuStudio-2.3.1";

/// True when an Application string is safe for Wonderprint-Orca 2.3.0.x / 2.3.1.
///
/// Accepts `BambuStudio-2.3.0` … `BambuStudio-2.3.2` (and the same with `OrcaSlicer-`).
/// Rejects MakerWorld / Bambu 02.xx builds (`02.06.00.51`, `02.07.01.62`, …).
pub fn is_wonderprint_safe_application(application: &str) -> bool {
    let rest = application
        .strip_prefix("BambuStudio-")
        .or_else(|| application.strip_prefix("OrcaSlicer-"))
        .unwrap_or("")
        .trim();
    if rest.is_empty() {
        return false;
    }
    // Leading-zero Bambu style (02.06.00.51) is always treated as foreign/new.
    if rest.starts_with('0') && rest.len() > 1 && rest.as_bytes().get(1) != Some(&b'.') {
        return false;
    }
    let mut parts = rest.split(|c| c == '.' || c == '-');
    let maj: u32 = match parts.next().and_then(|p| p.parse().ok()) {
        Some(v) => v,
        None => return false,
    };
    let min: u32 = match parts.next().and_then(|p| p.parse().ok()) {
        Some(v) => v,
        None => return false,
    };
    let pat: u32 = parts.next().and_then(|p| p.parse().ok()).unwrap_or(0);
    // Compatible window for Wonderprint 2.3.0.1: maj=2, min=3, patch ≤ 2.
    maj == 2 && min == 3 && pat <= 2
}

/// Choose Application to stamp: prefer a safe template value, else the Wonderprint default.
pub fn application_stamp_from_candidate(candidate: Option<&str>) -> String {
    match candidate.map(str::trim).filter(|s| !s.is_empty()) {
        Some(s) if is_wonderprint_safe_application(s) => s.to_string(),
        _ => DEFAULT_WONDERPRINT_APPLICATION.to_string(),
    }
}

/// Rewrite `<metadata name="Application">…</metadata>` in model XML.
///
/// Returns `None` if no Application element was found.
/// Returns `Some(rewritten)` when a replacement was applied (even if the value was
/// already identical, so callers can always write the buffer when needed).
pub fn rewrite_application_metadata(xml: &[u8], new_application: &str) -> Option<Vec<u8>> {
    let text = String::from_utf8_lossy(xml);
    let (idx, needle_len) = find_application_name_attr(&text)?;
    let after_name = idx + needle_len;
    let rest = &text[after_name..];
    let gt = rest.find('>')?;
    // Self-closing — nothing to replace.
    if rest[..gt].contains('/') {
        return None;
    }
    let content_start = after_name + gt + 1;
    let after_content = &text[content_start..];
    let end_rel = after_content.find("</")?;
    let end_abs = content_start + end_rel;
    let old_val = &text[content_start..end_abs];
    if old_val == new_application {
        return Some(xml.to_vec());
    }
    let mut out = String::with_capacity(text.len() + new_application.len());
    out.push_str(&text[..content_start]);
    out.push_str(new_application);
    out.push_str(&text[end_abs..]);
    Some(out.into_bytes())
}

/// Ensure root (or any) model XML has Application set to `new_application`.
///
/// Rewrites an existing tag, or injects one immediately after the opening `<model …>` tag.
pub fn ensure_application_metadata(xml: &[u8], new_application: &str) -> Vec<u8> {
    if let Some(rewritten) = rewrite_application_metadata(xml, new_application) {
        return rewritten;
    }
    inject_application_metadata(xml, new_application).unwrap_or_else(|| xml.to_vec())
}

/// Read Application metadata from model XML bytes, if present.
pub fn read_application_metadata(xml: &[u8]) -> Option<String> {
    let text = String::from_utf8_lossy(xml);
    let (idx, needle_len) = find_application_name_attr(&text)?;
    let after = &text[idx + needle_len..];
    let gt = after.find('>')?;
    if after[..gt].contains('/') {
        return None;
    }
    let rest = &after[gt + 1..];
    let end = rest.find("</")?;
    Some(rest[..end].trim().to_string())
}

fn find_application_name_attr(text: &str) -> Option<(usize, usize)> {
    // Double-quoted is the Bambu/Orca norm; accept single quotes too.
    const CANDIDATES: &[&str] = &["name=\"Application\"", "name='Application'"];
    let mut best: Option<(usize, usize)> = None;
    for needle in CANDIDATES {
        if let Some(idx) = text.find(needle) {
            match best {
                Some((bi, _)) if bi <= idx => {}
                _ => best = Some((idx, needle.len())),
            }
        }
    }
    best
}

fn inject_application_metadata(xml: &[u8], new_application: &str) -> Option<Vec<u8>> {
    let text = String::from_utf8_lossy(xml);
    // Find opening <model …> (with or without attributes).
    let lower = text.to_ascii_lowercase();
    let start = lower.find("<model")?;
    let after = &text[start..];
    let gt = after.find('>')?;
    // Self-closing <model …/> — rare; skip inject.
    if after[..gt].contains('/') {
        return None;
    }
    let insert_at = start + gt + 1;
    let injection = format!("\n <metadata name=\"Application\">{new_application}</metadata>");
    let mut out = String::with_capacity(text.len() + injection.len());
    out.push_str(&text[..insert_at]);
    out.push_str(&injection);
    out.push_str(&text[insert_at..]);
    Some(out.into_bytes())
}

#[cfg(test)]
#[allow(non_snake_case)]
mod tests {
    use super::*;

    #[test]
    fn rewrite_application__bambu_27_to_wonderprint__updates_value() {
        let xml = br#"<?xml version="1.0"?>
<model>
 <metadata name="Application">BambuStudio-02.07.01.62</metadata>
 <metadata name="Title">x</metadata>
</model>"#;
        let out = rewrite_application_metadata(xml, "BambuStudio-2.3.1").expect("rewrite");
        let s = String::from_utf8_lossy(&out);
        assert!(s.contains("BambuStudio-2.3.1"));
        assert!(!s.contains("02.07.01.62"));
        assert_eq!(
            read_application_metadata(&out).as_deref(),
            Some("BambuStudio-2.3.1")
        );
    }

    #[test]
    fn rewrite_application__single_quotes__updates_value() {
        let xml = br#"<?xml version="1.0"?>
<model>
 <metadata name='Application'>BambuStudio-02.06.00.51</metadata>
</model>"#;
        let out = rewrite_application_metadata(xml, "BambuStudio-2.3.1").expect("rewrite");
        assert_eq!(
            read_application_metadata(&out).as_deref(),
            Some("BambuStudio-2.3.1")
        );
    }

    #[test]
    fn rewrite_application__missing__returns_none() {
        let xml = br#"<?xml version="1.0"?><model><resources/></model>"#;
        assert!(rewrite_application_metadata(xml, "BambuStudio-2.3.1").is_none());
    }

    #[test]
    fn ensure_application__missing__injects_tag() {
        let xml = br#"<?xml version="1.0"?><model unit="millimeter"><resources/></model>"#;
        let out = ensure_application_metadata(xml, "BambuStudio-2.3.1");
        assert_eq!(
            read_application_metadata(&out).as_deref(),
            Some("BambuStudio-2.3.1")
        );
        assert!(String::from_utf8_lossy(&out).contains("<resources/>"));
    }

    #[test]
    fn is_safe__rejects_bambu_02_builds() {
        assert!(!is_wonderprint_safe_application(
            "BambuStudio-02.06.00.51"
        ));
        assert!(!is_wonderprint_safe_application(
            "BambuStudio-02.07.01.62"
        ));
        assert!(is_wonderprint_safe_application("BambuStudio-2.3.1"));
        assert!(is_wonderprint_safe_application("BambuStudio-2.3.0"));
        assert!(is_wonderprint_safe_application("BambuStudio-2.3.2"));
        assert!(!is_wonderprint_safe_application("BambuStudio-2.4.0"));
        assert!(!is_wonderprint_safe_application("BambuStudio-2.6.0.51"));
    }

    #[test]
    fn stamp_from_candidate__falls_back_on_unsafe() {
        assert_eq!(
            application_stamp_from_candidate(Some("BambuStudio-02.06.00.51")),
            DEFAULT_WONDERPRINT_APPLICATION
        );
        assert_eq!(
            application_stamp_from_candidate(Some("BambuStudio-2.3.1")),
            "BambuStudio-2.3.1"
        );
        assert_eq!(
            application_stamp_from_candidate(None),
            DEFAULT_WONDERPRINT_APPLICATION
        );
    }
}
