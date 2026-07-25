//! OPC package hygiene: PartName/Target normalization and orphan strip after member delete.
//!
//! Constraint **C3**: before comparing Override PartName / Relationship Target against
//! stripped or injected member sets, normalize paths (slash, strip leading `/`, collapse
//! `//`, percent-decode `%XX`).

use std::collections::{BTreeMap, BTreeSet};
use std::io::Cursor;

use quick_xml::Reader;
use quick_xml::Writer;
use quick_xml::XmlVersion;
use quick_xml::events::{BytesStart, Event};

use crate::error::{Error, Result};
use crate::paths::normalize_zip_path;

/// Well-known Content_Types member.
pub const CONTENT_TYPES: &str = "[Content_Types].xml";
/// Package-level relationships part.
pub const PACKAGE_RELS: &str = "_rels/.rels";

/// MIME for 3MF model parts (Default Extension="model" / Overrides).
pub const MIME_3DMODEL: &str = "application/vnd.ms-package.3dmanufacturing-3dmodel+xml";
/// MIME for relationship parts.
pub const MIME_RELS: &str = "application/vnd.openxmlformats-package.relationships+xml";

/// Normalize an OPC PartName or Relationship Target for membership compare (**C3**).
///
/// Steps:
/// 1. Backslash → `/`
/// 2. Percent-decode `%XX` (invalid sequences left as-is)
/// 3. Strip leading `/`
/// 4. Collapse `//`
pub fn normalize_opc_part_name(s: &str) -> String {
    let with_fwd = s.replace('\\', "/");
    let decoded = percent_decode(&with_fwd);
    normalize_zip_path(&decoded)
}

/// Percent-decode `%XX` sequences. Invalid hex pairs are left verbatim.
fn percent_decode(input: &str) -> String {
    let bytes = input.as_bytes();
    let mut out: Vec<u8> = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%'
            && i + 2 < bytes.len()
            && let (Some(hi), Some(lo)) = (from_hex(bytes[i + 1]), from_hex(bytes[i + 2]))
        {
            out.push((hi << 4) | lo);
            i += 3;
            continue;
        }
        out.push(bytes[i]);
        i += 1;
    }
    match String::from_utf8(out) {
        Ok(s) => s,
        Err(e) => String::from_utf8_lossy(e.as_bytes()).into_owned(),
    }
}

fn from_hex(b: u8) -> Option<u8> {
    match b {
        b'0'..=b'9' => Some(b - b'0'),
        b'a'..=b'f' => Some(b - b'a' + 10),
        b'A'..=b'F' => Some(b - b'A' + 10),
        _ => None,
    }
}

/// Parent directory of a `.rels` part, used to resolve relative Targets.
///
/// - `_rels/.rels` → `""` (package root)
/// - `3D/_rels/3dmodel.model.rels` → `"3D"`
pub fn rels_parent_dir(rels_member: &str) -> String {
    let name = normalize_zip_path(rels_member);
    // Expect `.../_rels/<file>.rels` → parent is dirname of `_rels`
    if let Some(idx) = name.rfind("/_rels/") {
        return name[..idx].to_string();
    }
    if name.starts_with("_rels/") {
        return String::new();
    }
    // Fallback: dirname of the rels file itself
    name.rsplit_once('/')
        .map(|(p, _)| p.to_string())
        .unwrap_or_default()
}

/// Resolve a Relationship Target to a package-root-relative normalized path.
///
/// Absolute targets (`/3D/...`) normalize from package root. Relative targets join
/// against the parent of the `_rels` folder that contains the relationships part.
pub fn resolve_relationship_target(target: &str, rels_parent: &str) -> String {
    let trimmed = target.trim();
    // Absolute if original (pre-normalize) starts with / or \ after optional whitespace
    let is_absolute = trimmed.starts_with('/') || trimmed.starts_with('\\');
    let normalized = normalize_opc_part_name(trimmed);
    if is_absolute || rels_parent.is_empty() {
        return normalized;
    }
    normalize_zip_path(&format!("{rels_parent}/{normalized}"))
}

/// True when the member is `[Content_Types].xml`.
pub fn is_content_types_member(name: &str) -> bool {
    normalize_zip_path(name) == CONTENT_TYPES
}

/// True when the member is a package or part relationships document (`*.rels`).
pub fn is_rels_member(name: &str) -> bool {
    let n = normalize_zip_path(name);
    n.ends_with(".rels")
}

/// Drop Override rows whose PartName is in `stripped`, keep Defaults (and other Overrides).
///
/// Optionally drops the `gcode` Default when no remaining members use that extension
/// (`remaining_extensions` is `None` → keep all Defaults).
pub fn strip_content_types_orphans(
    xml: &[u8],
    stripped: &BTreeSet<String>,
    remaining_extensions: Option<&BTreeSet<String>>,
) -> Result<Vec<u8>> {
    rewrite_content_types(
        xml,
        |part_name| {
            let n = normalize_opc_part_name(part_name);
            !stripped.contains(&n)
        },
        remaining_extensions,
    )
}

/// Drop Relationship rows whose resolved Target is in `stripped`.
pub fn strip_rels_orphans(
    xml: &[u8],
    stripped: &BTreeSet<String>,
    rels_parent: &str,
) -> Result<Vec<u8>> {
    rewrite_relationships(xml, |target| {
        let resolved = resolve_relationship_target(target, rels_parent);
        !stripped.contains(&resolved)
    })
}

/// Merge source Content_Types into a template base for S2 inject (**C1**).
///
/// - Union Defaults by extension (template wins on conflict; add source-only extensions).
/// - Keep template Overrides for non-stripped, non-overwritten parts.
/// - For each `injected` path, copy source Override when present; ensure Default for
///   the file extension exists (synthesize model Default if needed).
pub fn merge_content_types_for_inject(
    template_xml: Option<&[u8]>,
    source_xml: Option<&[u8]>,
    injected: &BTreeSet<String>,
    stripped: &BTreeSet<String>,
) -> Result<Vec<u8>> {
    let mut defaults: BTreeMap<String, String> = BTreeMap::new();
    let mut overrides: BTreeMap<String, String> = BTreeMap::new();

    if let Some(xml) = template_xml {
        parse_content_types_into(xml, &mut defaults, &mut overrides)?;
    }
    let mut source_defaults: BTreeMap<String, String> = BTreeMap::new();
    let mut source_overrides: BTreeMap<String, String> = BTreeMap::new();
    if let Some(xml) = source_xml {
        parse_content_types_into(xml, &mut source_defaults, &mut source_overrides)?;
    }

    // Union Defaults: template first, add missing from source.
    for (ext, ct) in source_defaults {
        defaults.entry(ext).or_insert(ct);
    }

    // Drop stripped overrides from template set.
    overrides.retain(|part, _| {
        let n = normalize_opc_part_name(part);
        !stripped.contains(&n)
    });

    // Injected paths: copy Override from source when present; ensure extension Default.
    let mut need_model_default = false;
    for path in injected {
        let n = normalize_zip_path(path);
        if stripped.contains(&n) {
            continue;
        }
        // Prefer source Override matching this part (try with and without leading slash keys).
        let ov = source_overrides
            .get(&n)
            .or_else(|| source_overrides.get(&format!("/{n}")))
            .cloned()
            .or_else(|| {
                // Scan source overrides with normalized keys
                source_overrides
                    .iter()
                    .find(|(k, _)| normalize_opc_part_name(k) == n)
                    .map(|(_, v)| v.clone())
            });
        if let Some(ct) = ov {
            overrides.insert(n.clone(), ct);
        }
        if let Some(ext) = extension_of(&n) {
            if ext.eq_ignore_ascii_case("model")
                && !defaults.keys().any(|e| e.eq_ignore_ascii_case("model"))
            {
                need_model_default = true;
            } else if !defaults.keys().any(|e| e.eq_ignore_ascii_case(&ext)) {
                // Unknown extension without Default — synthesize common ones.
                if ext.eq_ignore_ascii_case("model") {
                    need_model_default = true;
                } else if ext.eq_ignore_ascii_case("png") {
                    defaults.insert("png".into(), "image/png".into());
                } else if ext.eq_ignore_ascii_case("rels") {
                    defaults.insert("rels".into(), MIME_RELS.into());
                }
            }
        }
    }
    if need_model_default {
        defaults
            .entry("model".into())
            .or_insert_with(|| MIME_3DMODEL.to_string());
    }
    // Always ensure rels Default for a valid OPC package.
    defaults
        .entry("rels".into())
        .or_insert_with(|| MIME_RELS.to_string());

    // Drop Overrides whose part is not among remaining members if we only want inject+template
    // — keep template overrides for non-3D metadata still present.
    serialize_content_types(&defaults, &overrides)
}

/// Merge relationship documents: keep base relationships, add any from `add_from` whose
/// resolved Target is in `injected` and not already present (by resolved target).
pub fn merge_relationships(
    base_xml: Option<&[u8]>,
    add_from_xml: Option<&[u8]>,
    rels_parent: &str,
    injected: &BTreeSet<String>,
    stripped: &BTreeSet<String>,
) -> Result<Vec<u8>> {
    let mut kept: Vec<RelRow> = Vec::new();
    let mut seen_targets: BTreeSet<String> = BTreeSet::new();
    let mut seen_ids: BTreeSet<String> = BTreeSet::new();

    if let Some(xml) = base_xml {
        for row in parse_relationships(xml)? {
            let resolved = resolve_relationship_target(&row.target, rels_parent);
            if stripped.contains(&resolved) {
                continue;
            }
            seen_targets.insert(resolved);
            seen_ids.insert(row.id.clone());
            kept.push(row);
        }
    }
    if let Some(xml) = add_from_xml {
        for mut row in parse_relationships(xml)? {
            let resolved = resolve_relationship_target(&row.target, rels_parent);
            if stripped.contains(&resolved) {
                continue;
            }
            if !injected.contains(&resolved) {
                continue;
            }
            if seen_targets.contains(&resolved) {
                continue;
            }
            // Avoid Id collision
            if seen_ids.contains(&row.id) {
                let mut n = 1u32;
                loop {
                    let candidate = format!("wmRel{n}");
                    if !seen_ids.contains(&candidate) {
                        row.id = candidate;
                        break;
                    }
                    n += 1;
                }
            }
            seen_targets.insert(resolved);
            seen_ids.insert(row.id.clone());
            kept.push(row);
        }
    }
    serialize_relationships(&kept)
}

/// Ensure package relationships include a 3dmodel relationship for the root model when
/// injected and missing.
pub fn ensure_root_model_relationship(
    rels_xml: Option<&[u8]>,
    root_model: &str,
) -> Result<Vec<u8>> {
    let root = normalize_zip_path(root_model);
    let mut rows = if let Some(xml) = rels_xml {
        parse_relationships(xml)?
    } else {
        Vec::new()
    };
    let has = rows.iter().any(|r| {
        let resolved = resolve_relationship_target(&r.target, "");
        resolved == root
    });
    if !has {
        rows.push(RelRow {
            id: "wm3dmodel".into(),
            rel_type: "http://schemas.microsoft.com/3dmanufacturing/2013/01/3dmodel".into(),
            target: format!("/{root}"),
            target_mode: None,
        });
    }
    serialize_relationships(&rows)
}

// --- internal Content_Types parse/write ---

fn parse_content_types_into(
    xml: &[u8],
    defaults: &mut BTreeMap<String, String>,
    overrides: &mut BTreeMap<String, String>,
) -> Result<()> {
    let mut reader = Reader::from_reader(Cursor::new(xml));
    reader.config_mut().trim_text(true);
    let mut buf = Vec::new();
    loop {
        match reader.read_event_into(&mut buf) {
            Ok(Event::Start(e)) | Ok(Event::Empty(e)) => {
                let local = local_name(e.name().as_ref()).to_vec();
                if local == b"Default" {
                    let mut ext = None;
                    let mut ct = None;
                    for attr in e.attributes() {
                        let attr = attr.map_err(|err| {
                            Error::msg(format!("Content_Types attribute parse failed: {err}"))
                        })?;
                        let an = local_name(attr.key.as_ref()).to_vec();
                        let av = attr_value(&attr)?;
                        if an == b"Extension" {
                            ext = Some(av);
                        } else if an == b"ContentType" {
                            ct = Some(av);
                        }
                    }
                    if let (Some(ext), Some(ct)) = (ext, ct) {
                        defaults.insert(ext, ct);
                    }
                } else if local == b"Override" {
                    let mut part = None;
                    let mut ct = None;
                    for attr in e.attributes() {
                        let attr = attr.map_err(|err| {
                            Error::msg(format!("Content_Types attribute parse failed: {err}"))
                        })?;
                        let an = local_name(attr.key.as_ref()).to_vec();
                        let av = attr_value(&attr)?;
                        if an == b"PartName" {
                            part = Some(av);
                        } else if an == b"ContentType" {
                            ct = Some(av);
                        }
                    }
                    if let (Some(part), Some(ct)) = (part, ct) {
                        let key = normalize_opc_part_name(&part);
                        overrides.insert(key, ct);
                    }
                }
            }
            Ok(Event::Eof) => break,
            Err(e) => return Err(Error::xml("[Content_Types].xml", e)),
            _ => {}
        }
        buf.clear();
    }
    Ok(())
}

fn rewrite_content_types(
    xml: &[u8],
    keep_override: impl Fn(&str) -> bool,
    remaining_extensions: Option<&BTreeSet<String>>,
) -> Result<Vec<u8>> {
    let mut reader = Reader::from_reader(Cursor::new(xml));
    reader.config_mut().trim_text(false);
    let mut writer = Writer::new(Cursor::new(Vec::with_capacity(xml.len())));
    let mut buf = Vec::new();

    loop {
        match reader.read_event_into(&mut buf) {
            Ok(Event::Start(e)) => {
                if local_name(e.name().as_ref()) == b"Override" {
                    if !should_keep_override(&e, &keep_override)? {
                        // Skip until matching End (or treat Empty separately — Start means not empty).
                        skip_until_end(&mut reader, b"Override")?;
                    } else {
                        writer
                            .write_event(Event::Start(e.into_owned()))
                            .map_err(|err| {
                                Error::msg(format!("Content_Types write failed: {err}"))
                            })?;
                    }
                } else if local_name(e.name().as_ref()) == b"Default" {
                    if should_keep_default(&e, remaining_extensions)? {
                        writer
                            .write_event(Event::Start(e.into_owned()))
                            .map_err(|err| {
                                Error::msg(format!("Content_Types write failed: {err}"))
                            })?;
                    } else {
                        skip_until_end(&mut reader, b"Default")?;
                    }
                } else {
                    writer
                        .write_event(Event::Start(e.into_owned()))
                        .map_err(|err| Error::msg(format!("Content_Types write failed: {err}")))?;
                }
            }
            Ok(Event::Empty(e)) => {
                if local_name(e.name().as_ref()) == b"Override" {
                    if should_keep_override(&e, &keep_override)? {
                        writer
                            .write_event(Event::Empty(e.into_owned()))
                            .map_err(|err| {
                                Error::msg(format!("Content_Types write failed: {err}"))
                            })?;
                    }
                } else if local_name(e.name().as_ref()) == b"Default" {
                    if should_keep_default(&e, remaining_extensions)? {
                        writer
                            .write_event(Event::Empty(e.into_owned()))
                            .map_err(|err| {
                                Error::msg(format!("Content_Types write failed: {err}"))
                            })?;
                    }
                } else {
                    writer
                        .write_event(Event::Empty(e.into_owned()))
                        .map_err(|err| Error::msg(format!("Content_Types write failed: {err}")))?;
                }
            }
            Ok(Event::End(e)) => {
                writer
                    .write_event(Event::End(e.into_owned()))
                    .map_err(|err| Error::msg(format!("Content_Types write failed: {err}")))?;
            }
            Ok(Event::Text(t)) => {
                writer
                    .write_event(Event::Text(t.into_owned()))
                    .map_err(|err| Error::msg(format!("Content_Types write failed: {err}")))?;
            }
            Ok(Event::CData(c)) => {
                writer
                    .write_event(Event::CData(c.into_owned()))
                    .map_err(|err| Error::msg(format!("Content_Types write failed: {err}")))?;
            }
            Ok(Event::Comment(c)) => {
                writer
                    .write_event(Event::Comment(c.into_owned()))
                    .map_err(|err| Error::msg(format!("Content_Types write failed: {err}")))?;
            }
            Ok(Event::Decl(d)) => {
                writer
                    .write_event(Event::Decl(d.into_owned()))
                    .map_err(|err| Error::msg(format!("Content_Types write failed: {err}")))?;
            }
            Ok(Event::PI(p)) => {
                writer
                    .write_event(Event::PI(p.into_owned()))
                    .map_err(|err| Error::msg(format!("Content_Types write failed: {err}")))?;
            }
            Ok(Event::DocType(d)) => {
                writer
                    .write_event(Event::DocType(d.into_owned()))
                    .map_err(|err| Error::msg(format!("Content_Types write failed: {err}")))?;
            }
            Ok(Event::GeneralRef(g)) => {
                writer
                    .write_event(Event::GeneralRef(g.into_owned()))
                    .map_err(|err| Error::msg(format!("Content_Types write failed: {err}")))?;
            }
            Ok(Event::Eof) => break,
            Err(e) => return Err(Error::xml("[Content_Types].xml", e)),
        }
        buf.clear();
    }
    Ok(writer.into_inner().into_inner())
}

fn should_keep_override(e: &BytesStart<'_>, keep: &impl Fn(&str) -> bool) -> Result<bool> {
    for attr in e.attributes() {
        let attr =
            attr.map_err(|err| Error::msg(format!("Content_Types attribute parse failed: {err}")))?;
        if local_name(attr.key.as_ref()) == b"PartName" {
            let v = attr_value(&attr)?;
            return Ok(keep(&v));
        }
    }
    Ok(true)
}

fn should_keep_default(
    e: &BytesStart<'_>,
    remaining_extensions: Option<&BTreeSet<String>>,
) -> Result<bool> {
    let Some(remaining) = remaining_extensions else {
        return Ok(true);
    };
    for attr in e.attributes() {
        let attr =
            attr.map_err(|err| Error::msg(format!("Content_Types attribute parse failed: {err}")))?;
        if local_name(attr.key.as_ref()) == b"Extension" {
            let ext = attr_value(&attr)?;
            // Always keep rels; drop extension only when no remaining members use it.
            if ext.eq_ignore_ascii_case("rels") {
                return Ok(true);
            }
            let keep = remaining.iter().any(|r| r.eq_ignore_ascii_case(&ext));
            return Ok(keep);
        }
    }
    Ok(true)
}

fn serialize_content_types(
    defaults: &BTreeMap<String, String>,
    overrides: &BTreeMap<String, String>,
) -> Result<Vec<u8>> {
    let mut out = String::from(
        r#"<?xml version="1.0" encoding="UTF-8"?>
<Types xmlns="http://schemas.openxmlformats.org/package/2006/content-types">
"#,
    );
    for (ext, ct) in defaults {
        out.push_str(&format!(
            r#"  <Default Extension="{}" ContentType="{}"/>
"#,
            xml_escape(ext),
            xml_escape(ct)
        ));
    }
    for (part, ct) in overrides {
        // OPC PartName is absolute with leading slash.
        let part_name = if part.starts_with('/') {
            part.clone()
        } else {
            format!("/{part}")
        };
        out.push_str(&format!(
            r#"  <Override PartName="{}" ContentType="{}"/>
"#,
            xml_escape(&part_name),
            xml_escape(ct)
        ));
    }
    out.push_str("</Types>\n");
    Ok(out.into_bytes())
}

// --- Relationships ---

#[derive(Debug, Clone)]
struct RelRow {
    id: String,
    rel_type: String,
    target: String,
    target_mode: Option<String>,
}

fn parse_relationships(xml: &[u8]) -> Result<Vec<RelRow>> {
    let mut reader = Reader::from_reader(Cursor::new(xml));
    reader.config_mut().trim_text(true);
    let mut rows = Vec::new();
    let mut buf = Vec::new();
    loop {
        match reader.read_event_into(&mut buf) {
            Ok(Event::Start(e)) | Ok(Event::Empty(e)) => {
                if local_name(e.name().as_ref()) == b"Relationship" {
                    let mut id = String::new();
                    let mut rel_type = String::new();
                    let mut target = String::new();
                    let mut target_mode = None;
                    for attr in e.attributes() {
                        let attr = attr.map_err(|err| {
                            Error::msg(format!("relationships attribute parse failed: {err}"))
                        })?;
                        let an = local_name(attr.key.as_ref());
                        let av = attr_value(&attr)?;
                        if an == b"Id" {
                            id = av;
                        } else if an == b"Type" {
                            rel_type = av;
                        } else if an == b"Target" {
                            target = av;
                        } else if an == b"TargetMode" {
                            target_mode = Some(av);
                        }
                    }
                    if !target.is_empty() {
                        rows.push(RelRow {
                            id,
                            rel_type,
                            target,
                            target_mode,
                        });
                    }
                }
            }
            Ok(Event::Eof) => break,
            Err(e) => return Err(Error::xml("relationships", e)),
            _ => {}
        }
        buf.clear();
    }
    Ok(rows)
}

fn rewrite_relationships(xml: &[u8], keep_target: impl Fn(&str) -> bool) -> Result<Vec<u8>> {
    let mut reader = Reader::from_reader(Cursor::new(xml));
    reader.config_mut().trim_text(false);
    let mut writer = Writer::new(Cursor::new(Vec::with_capacity(xml.len())));
    let mut buf = Vec::new();

    loop {
        match reader.read_event_into(&mut buf) {
            Ok(Event::Start(e)) => {
                if local_name(e.name().as_ref()) == b"Relationship" {
                    if relationship_keep(&e, &keep_target)? {
                        writer
                            .write_event(Event::Start(e.into_owned()))
                            .map_err(|err| Error::msg(format!("rels write failed: {err}")))?;
                    } else {
                        skip_until_end(&mut reader, b"Relationship")?;
                    }
                } else {
                    writer
                        .write_event(Event::Start(e.into_owned()))
                        .map_err(|err| Error::msg(format!("rels write failed: {err}")))?;
                }
            }
            Ok(Event::Empty(e)) => {
                if local_name(e.name().as_ref()) == b"Relationship" {
                    if relationship_keep(&e, &keep_target)? {
                        writer
                            .write_event(Event::Empty(e.into_owned()))
                            .map_err(|err| Error::msg(format!("rels write failed: {err}")))?;
                    }
                } else {
                    writer
                        .write_event(Event::Empty(e.into_owned()))
                        .map_err(|err| Error::msg(format!("rels write failed: {err}")))?;
                }
            }
            Ok(Event::End(e)) => {
                writer
                    .write_event(Event::End(e.into_owned()))
                    .map_err(|err| Error::msg(format!("rels write failed: {err}")))?;
            }
            Ok(Event::Text(t)) => {
                writer
                    .write_event(Event::Text(t.into_owned()))
                    .map_err(|err| Error::msg(format!("rels write failed: {err}")))?;
            }
            Ok(Event::CData(c)) => {
                writer
                    .write_event(Event::CData(c.into_owned()))
                    .map_err(|err| Error::msg(format!("rels write failed: {err}")))?;
            }
            Ok(Event::Comment(c)) => {
                writer
                    .write_event(Event::Comment(c.into_owned()))
                    .map_err(|err| Error::msg(format!("rels write failed: {err}")))?;
            }
            Ok(Event::Decl(d)) => {
                writer
                    .write_event(Event::Decl(d.into_owned()))
                    .map_err(|err| Error::msg(format!("rels write failed: {err}")))?;
            }
            Ok(Event::PI(p)) => {
                writer
                    .write_event(Event::PI(p.into_owned()))
                    .map_err(|err| Error::msg(format!("rels write failed: {err}")))?;
            }
            Ok(Event::DocType(d)) => {
                writer
                    .write_event(Event::DocType(d.into_owned()))
                    .map_err(|err| Error::msg(format!("rels write failed: {err}")))?;
            }
            Ok(Event::GeneralRef(g)) => {
                writer
                    .write_event(Event::GeneralRef(g.into_owned()))
                    .map_err(|err| Error::msg(format!("rels write failed: {err}")))?;
            }
            Ok(Event::Eof) => break,
            Err(e) => return Err(Error::xml("relationships", e)),
        }
        buf.clear();
    }
    Ok(writer.into_inner().into_inner())
}

fn relationship_keep(e: &BytesStart<'_>, keep: &impl Fn(&str) -> bool) -> Result<bool> {
    for attr in e.attributes() {
        let attr =
            attr.map_err(|err| Error::msg(format!("relationships attribute parse failed: {err}")))?;
        if local_name(attr.key.as_ref()) == b"Target" {
            let v = attr_value(&attr)?;
            return Ok(keep(&v));
        }
    }
    Ok(true)
}

fn serialize_relationships(rows: &[RelRow]) -> Result<Vec<u8>> {
    let mut out = String::from(
        r#"<?xml version="1.0" encoding="UTF-8"?>
<Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships">
"#,
    );
    for r in rows {
        if let Some(ref mode) = r.target_mode {
            out.push_str(&format!(
                r#"  <Relationship Id="{}" Type="{}" Target="{}" TargetMode="{}"/>
"#,
                xml_escape(&r.id),
                xml_escape(&r.rel_type),
                xml_escape(&r.target),
                xml_escape(mode)
            ));
        } else {
            out.push_str(&format!(
                r#"  <Relationship Id="{}" Type="{}" Target="{}"/>
"#,
                xml_escape(&r.id),
                xml_escape(&r.rel_type),
                xml_escape(&r.target)
            ));
        }
    }
    out.push_str("</Relationships>\n");
    Ok(out.into_bytes())
}

fn skip_until_end<R: std::io::BufRead>(reader: &mut Reader<R>, local: &[u8]) -> Result<()> {
    let mut depth = 1i32;
    let mut buf = Vec::new();
    while depth > 0 {
        match reader.read_event_into(&mut buf) {
            Ok(Event::Start(e)) => {
                if local_name(e.name().as_ref()) == local {
                    depth += 1;
                }
            }
            Ok(Event::End(e)) => {
                if local_name(e.name().as_ref()) == local {
                    depth -= 1;
                }
            }
            Ok(Event::Eof) => break,
            Err(e) => return Err(Error::xml("skip element", e)),
            _ => {}
        }
        buf.clear();
    }
    Ok(())
}

fn attr_value(attr: &quick_xml::events::attributes::Attribute<'_>) -> Result<String> {
    attr.normalized_value(XmlVersion::Implicit1_0)
        .map(|c| c.into_owned())
        .map_err(|err| Error::msg(format!("attribute normalize failed: {err}")))
}

fn local_name(qname: &[u8]) -> &[u8] {
    qname.rsplit(|b| *b == b':').next().unwrap_or(qname)
}

fn extension_of(path: &str) -> Option<String> {
    let name = path.rsplit('/').next().unwrap_or(path);
    name.rsplit_once('.').map(|(_, e)| e.to_string())
}

fn xml_escape(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
        .replace('\'', "&apos;")
}

/// Collect file extensions present among remaining (non-stripped) members.
pub fn remaining_extensions(
    members: impl IntoIterator<Item = impl AsRef<str>>,
) -> BTreeSet<String> {
    let mut set = BTreeSet::new();
    for m in members {
        if let Some(ext) = extension_of(m.as_ref()) {
            set.insert(ext.to_ascii_lowercase());
        }
    }
    set
}

#[cfg(test)]
#[allow(non_snake_case)]
mod tests {
    use super::*;

    #[test]
    fn opc__normalize_targets__slash_and_pct20() {
        assert_eq!(
            normalize_opc_part_name("/Metadata/plate_1.gcode"),
            "Metadata/plate_1.gcode"
        );
        assert_eq!(
            normalize_opc_part_name(r"Metadata\plate_1.gcode"),
            "Metadata/plate_1.gcode"
        );
        assert_eq!(
            normalize_opc_part_name("Metadata/my%20plate.gcode"),
            "Metadata/my plate.gcode"
        );
        assert_eq!(
            normalize_opc_part_name("/3D//Objects/a.model"),
            "3D/Objects/a.model"
        );
    }

    #[test]
    fn opc__resolve_target__absolute_and_relative() {
        assert_eq!(
            resolve_relationship_target("/Metadata/plate_1.gcode", "3D"),
            "Metadata/plate_1.gcode"
        );
        assert_eq!(
            resolve_relationship_target("Objects/a.model", "3D"),
            "3D/Objects/a.model"
        );
        assert_eq!(
            resolve_relationship_target("3D/3dmodel.model", ""),
            "3D/3dmodel.model"
        );
    }

    #[test]
    fn opc__strip_gcode__removes_override_and_rel() {
        let ct = br#"<?xml version="1.0"?>
<Types xmlns="http://schemas.openxmlformats.org/package/2006/content-types">
  <Default Extension="model" ContentType="application/vnd.ms-package.3dmanufacturing-3dmodel+xml"/>
  <Default Extension="gcode" ContentType="text/plain"/>
  <Override PartName="/3D/3dmodel.model" ContentType="application/vnd.ms-package.3dmanufacturing-3dmodel+xml"/>
  <Override PartName="/Metadata/plate_1.gcode" ContentType="text/plain"/>
  <Override PartName="/Metadata/custom_gcode_per_layer.xml" ContentType="text/xml"/>
</Types>
"#;
        let mut stripped = BTreeSet::new();
        stripped.insert("Metadata/plate_1.gcode".into());
        stripped.insert("Metadata/custom_gcode_per_layer.xml".into());
        let mut remaining = BTreeSet::new();
        remaining.insert("model".into());
        let out = strip_content_types_orphans(ct, &stripped, Some(&remaining)).expect("strip ct");
        let text = String::from_utf8(out).expect("utf8");
        assert!(
            !text.contains("plate_1.gcode"),
            "override must drop: {text}"
        );
        assert!(
            !text.contains("custom_gcode_per_layer"),
            "override must drop: {text}"
        );
        assert!(
            text.contains("3dmodel.model"),
            "keep model override: {text}"
        );
        assert!(
            !text.contains("Extension=\"gcode\""),
            "gcode Default should drop when no remaining gcode: {text}"
        );

        let rels = br#"<?xml version="1.0"?>
<Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships">
  <Relationship Id="r1" Type="http://example/3dmodel" Target="/3D/3dmodel.model"/>
  <Relationship Id="r2" Type="http://example/gcode" Target="/Metadata/plate_1.gcode"/>
  <Relationship Id="r3" Type="http://example/gcode" Target="Metadata\plate_1.gcode"/>
  <Relationship Id="r4" Type="http://example/gcode" Target="/Metadata/my%20plate.gcode"/>
</Relationships>
"#;
        stripped.insert("Metadata/my plate.gcode".into());
        let out = strip_rels_orphans(rels, &stripped, "").expect("strip rels");
        let text = String::from_utf8(out).expect("utf8");
        assert!(text.contains("3dmodel.model"), "keep model rel: {text}");
        assert!(
            !text.contains("plate_1.gcode"),
            "drop gcode targets: {text}"
        );
        assert!(
            !text.contains("my%20plate") && !text.contains("my plate"),
            "{text}"
        );
    }

    #[test]
    fn opc__merge_content_types__adds_model_default() {
        // Template has no model Default (empty 3D shell).
        let template = br#"<?xml version="1.0"?>
<Types xmlns="http://schemas.openxmlformats.org/package/2006/content-types">
  <Default Extension="rels" ContentType="application/vnd.openxmlformats-package.relationships+xml"/>
</Types>
"#;
        let source = br#"<?xml version="1.0"?>
<Types xmlns="http://schemas.openxmlformats.org/package/2006/content-types">
  <Default Extension="model" ContentType="application/vnd.ms-package.3dmanufacturing-3dmodel+xml"/>
  <Override PartName="/3D/Objects/object_1.model" ContentType="application/vnd.ms-package.3dmanufacturing-3dmodel+xml"/>
</Types>
"#;
        let mut injected = BTreeSet::new();
        injected.insert("3D/3dmodel.model".into());
        injected.insert("3D/Objects/object_1.model".into());
        let out = merge_content_types_for_inject(
            Some(template),
            Some(source),
            &injected,
            &BTreeSet::new(),
        )
        .expect("merge");
        let text = String::from_utf8(out).expect("utf8");
        assert!(
            text.contains("Extension=\"model\""),
            "must have model Default: {text}"
        );
        assert!(
            text.contains("object_1.model"),
            "must copy Override: {text}"
        );
    }
}
