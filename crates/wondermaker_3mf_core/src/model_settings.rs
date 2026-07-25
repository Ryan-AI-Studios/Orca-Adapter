//! Parse and remap `Metadata/model_settings.config` for plates, extruders, and slot maps.

use std::collections::BTreeMap;
use std::io::Cursor;

use quick_xml::Reader;
use quick_xml::Writer;
use quick_xml::XmlVersion;
use quick_xml::events::{BytesEnd, BytesStart, BytesText, Event};

use crate::error::{Error, Result};
use crate::slot_map::SlotMap;

/// Summary extracted from model_settings XML.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ModelSettingsSummary {
    /// Number of `<plate>` elements.
    pub plate_count: u32,
    /// Counts of 1-based extruder indices from `key="extruder"` metadata attrs.
    pub extruder_histogram: BTreeMap<u8, u32>,
}

impl ModelSettingsSummary {
    /// Sorted list of used 1-based extruder indices.
    pub fn used_extruders(&self) -> Vec<u8> {
        self.extruder_histogram.keys().copied().collect()
    }
}

/// Stats from extruder remapping.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ExtruderRemapStats {
    /// Existing extruder values rewritten via the map.
    pub rewritten: u32,
    /// Implicit slot-1 extruders injected when map(1) ≠ 1.
    pub injected: u32,
    /// Histogram of destination extruder values after remap (including injected).
    pub histogram_out: BTreeMap<u8, u32>,
}

/// Parse model_settings.config bytes for plate count + extruder histogram.
///
/// **DoD-7:** element and attribute qnames must be valid UTF-8. Invalid qnames
/// return `Error` on this path (including identity convert, which only parses
/// then raw-copies `model_settings.config`).
pub fn parse_model_settings(bytes: &[u8]) -> Result<ModelSettingsSummary> {
    let mut reader = Reader::from_reader(Cursor::new(bytes));
    reader.config_mut().trim_text(true);

    let mut summary = ModelSettingsSummary::default();
    let mut buf = Vec::new();

    loop {
        match reader.read_event_into(&mut buf) {
            Ok(Event::Start(e)) | Ok(Event::Empty(e)) => {
                validate_start_qnames(&e)?;
                let name = e.name();
                let local = local_name(name.as_ref());
                if local == b"plate" {
                    summary.plate_count += 1;
                }
                // Collect extruder from metadata elements:
                // <metadata key="extruder" value="N"/>
                if local == b"metadata" {
                    let mut key: Option<String> = None;
                    let mut value: Option<String> = None;
                    for attr_result in e.attributes() {
                        let attr = attr_result.map_err(|err| {
                            Error::msg(format!("model_settings attribute parse failed: {err}"))
                        })?;
                        let an = local_name(attr.key.as_ref());
                        let av = attr
                            .normalized_value(XmlVersion::Implicit1_0)
                            .map_err(|err| {
                                Error::msg(format!(
                                    "model_settings attribute normalize failed: {err}"
                                ))
                            })?
                            .into_owned();
                        if an == b"key" {
                            key = Some(av);
                        } else if an == b"value" {
                            value = Some(av);
                        }
                    }
                    // Non-numeric extruder values are ignored (not a hard parse failure).
                    if key.as_deref() == Some("extruder")
                        && let Some(v) = value
                        && let Ok(n) = v.parse::<u8>()
                        && n >= 1
                    {
                        *summary.extruder_histogram.entry(n).or_insert(0) += 1;
                    }
                }
            }
            Ok(Event::End(e)) => {
                validate_end_qname(&e)?;
            }
            Ok(Event::Eof) => break,
            Err(e) => return Err(Error::xml("model_settings.config", e)),
            _ => {}
        }
        buf.clear();
    }

    Ok(summary)
}

/// Context frame for object/part containers while remapping extruders.
#[derive(Debug, Clone)]
struct ContainerCtx {
    /// True when this container can carry a filament extruder.
    filament_bearing: bool,
    /// Explicit extruder metadata was seen inside this container (direct child level).
    has_extruder: bool,
}

/// Rewrite `key="extruder"` values via `map`, and inject implicit slot 1 when needed.
///
/// **C2:** quick-xml Reader/Writer only.
/// **C3:** missing extruder on filament-bearing normal_part/object = source slot 1;
/// inject dest when `map(1) ≠ 1`.
pub fn remap_model_settings_extruders(
    xml: &[u8],
    map: &SlotMap,
) -> Result<(Vec<u8>, ExtruderRemapStats)> {
    let mut reader = Reader::from_reader(Cursor::new(xml));
    reader.config_mut().trim_text(false);

    let mut writer = Writer::new(Cursor::new(Vec::with_capacity(xml.len())));
    let mut stats = ExtruderRemapStats::default();
    let mut stack: Vec<ContainerCtx> = Vec::new();
    let mut buf = Vec::new();

    // Pending injection: when we see End of filament-bearing container without extruder,
    // write inject metadata then End.
    loop {
        match reader.read_event_into(&mut buf) {
            Ok(Event::Start(e)) => {
                let local = local_name(e.name().as_ref()).to_vec();
                handle_container_start(&local, &e, &mut stack)?;

                if local == b"metadata" {
                    let (new_event, rewritten_dest) =
                        maybe_rewrite_extruder_metadata(e, map, &mut stats)?;
                    if let Some(dest) = rewritten_dest {
                        mark_parent_has_extruder(&mut stack, dest);
                    }
                    writer
                        .write_event(new_event)
                        .map_err(|err| Error::msg(format!("model_settings write failed: {err}")))?;
                } else {
                    writer
                        .write_event(Event::Start(e.into_owned()))
                        .map_err(|err| Error::msg(format!("model_settings write failed: {err}")))?;
                }
            }
            Ok(Event::Empty(e)) => {
                let local = local_name(e.name().as_ref()).to_vec();
                // Self-closing part/object (rare) or metadata.
                if local == b"metadata" {
                    let (new_event, rewritten_dest) =
                        maybe_rewrite_extruder_metadata_empty(e, map, &mut stats)?;
                    if let Some(dest) = rewritten_dest {
                        mark_parent_has_extruder(&mut stack, dest);
                    }
                    writer
                        .write_event(new_event)
                        .map_err(|err| Error::msg(format!("model_settings write failed: {err}")))?;
                } else if local == b"part" || local == b"object" {
                    // Self-closing container: may need inject as sibling content — cannot
                    // inject into empty element. Expand to start+inject+end if needed.
                    let ctx = container_from_start(&local, &e)?;
                    if ctx.filament_bearing && !ctx.has_extruder && map.map_slot(1) != 1 {
                        let dest = map.map_slot(1);
                        let name_str = qname_to_str(e.name().as_ref())?;
                        let mut start = BytesStart::new(name_str.clone());
                        for attr_result in e.attributes() {
                            let attr = attr_result.map_err(|err| {
                                Error::msg(format!("model_settings attribute parse failed: {err}"))
                            })?;
                            let k = qname_to_str(attr.key.as_ref())?;
                            let v = attr
                                .normalized_value(XmlVersion::Implicit1_0)
                                .map_err(|err| {
                                    Error::msg(format!(
                                        "model_settings attribute normalize failed: {err}"
                                    ))
                                })?
                                .into_owned();
                            start.push_attribute((k.as_str(), v.as_str()));
                        }
                        writer.write_event(Event::Start(start)).map_err(|err| {
                            Error::msg(format!("model_settings write failed: {err}"))
                        })?;
                        write_extruder_metadata(&mut writer, dest)?;
                        stats.injected += 1;
                        *stats.histogram_out.entry(dest).or_insert(0) += 1;
                        writer
                            .write_event(Event::End(BytesEnd::new(name_str)))
                            .map_err(|err| {
                                Error::msg(format!("model_settings write failed: {err}"))
                            })?;
                    } else {
                        writer
                            .write_event(Event::Empty(e.into_owned()))
                            .map_err(|err| {
                                Error::msg(format!("model_settings write failed: {err}"))
                            })?;
                    }
                } else {
                    writer
                        .write_event(Event::Empty(e.into_owned()))
                        .map_err(|err| Error::msg(format!("model_settings write failed: {err}")))?;
                }
            }
            Ok(Event::End(e)) => {
                let local = local_name(e.name().as_ref()).to_vec();
                let name_str = qname_to_str(e.name().as_ref())?;

                // Pop matching container and maybe inject before End.
                if (local == b"part" || local == b"object")
                    && let Some(ctx) = stack.pop()
                    && ctx.filament_bearing
                    && !ctx.has_extruder
                {
                    let dest = map.map_slot(1);
                    if dest != 1 {
                        write_extruder_metadata(&mut writer, dest)?;
                        stats.injected += 1;
                        *stats.histogram_out.entry(dest).or_insert(0) += 1;
                    }
                    // Identity for slot 1: implicit remains implicit (no inject).
                }

                writer
                    .write_event(Event::End(BytesEnd::new(name_str)))
                    .map_err(|err| Error::msg(format!("model_settings write failed: {err}")))?;
            }
            Ok(Event::Text(t)) => {
                let content = t.decode().map_err(|err| {
                    Error::msg(format!("model_settings text decode failed: {err}"))
                })?;
                writer
                    .write_event(Event::Text(BytesText::new(&content)))
                    .map_err(|err| Error::msg(format!("model_settings write failed: {err}")))?;
            }
            Ok(Event::CData(c)) => {
                writer
                    .write_event(Event::CData(c.into_owned()))
                    .map_err(|err| Error::msg(format!("model_settings write failed: {err}")))?;
            }
            Ok(Event::Comment(c)) => {
                writer
                    .write_event(Event::Comment(c.into_owned()))
                    .map_err(|err| Error::msg(format!("model_settings write failed: {err}")))?;
            }
            Ok(Event::Decl(d)) => {
                writer
                    .write_event(Event::Decl(d.into_owned()))
                    .map_err(|err| Error::msg(format!("model_settings write failed: {err}")))?;
            }
            Ok(Event::PI(p)) => {
                writer
                    .write_event(Event::PI(p.into_owned()))
                    .map_err(|err| Error::msg(format!("model_settings write failed: {err}")))?;
            }
            Ok(Event::DocType(d)) => {
                writer
                    .write_event(Event::DocType(d.into_owned()))
                    .map_err(|err| Error::msg(format!("model_settings write failed: {err}")))?;
            }
            Ok(Event::GeneralRef(g)) => {
                writer
                    .write_event(Event::GeneralRef(g.into_owned()))
                    .map_err(|err| Error::msg(format!("model_settings write failed: {err}")))?;
            }
            Ok(Event::Eof) => break,
            Err(e) => return Err(Error::xml("model_settings extruder remap", e)),
        }
        buf.clear();
    }

    let out = writer.into_inner().into_inner();
    Ok((out, stats))
}

fn handle_container_start(
    local: &[u8],
    e: &BytesStart<'_>,
    stack: &mut Vec<ContainerCtx>,
) -> Result<()> {
    if local == b"part" || local == b"object" {
        stack.push(container_from_start(local, e)?);
    }
    Ok(())
}

fn container_from_start(local: &[u8], e: &BytesStart<'_>) -> Result<ContainerCtx> {
    let is_part = local == b"part";

    let mut subtype: Option<String> = None;
    for attr_result in e.attributes() {
        let attr = attr_result
            .map_err(|err| Error::msg(format!("model_settings attribute parse failed: {err}")))?;
        if local_name(attr.key.as_ref()) == b"subtype" {
            let v = attr
                .normalized_value(XmlVersion::Implicit1_0)
                .map_err(|err| {
                    Error::msg(format!("model_settings attribute normalize failed: {err}"))
                })?
                .into_owned();
            subtype = Some(v);
        }
    }

    let filament_bearing = if is_part {
        match subtype.as_deref() {
            // Skip non-filament subtypes when possible.
            Some("negative_part")
            | Some("modifier")
            | Some("support_enforcer")
            | Some("support_blocker") => false,
            // normal_part, missing subtype, or unknown → treat as filament-bearing.
            _ => true,
        }
    } else {
        // object
        true
    };

    Ok(ContainerCtx {
        filament_bearing,
        has_extruder: false,
    })
}

fn mark_parent_has_extruder(stack: &mut [ContainerCtx], _dest: u8) {
    if let Some(ctx) = stack.last_mut() {
        ctx.has_extruder = true;
    }
}

/// Rewrite extruder metadata on a Start event. Returns event + optional dest value counted.
fn maybe_rewrite_extruder_metadata(
    e: BytesStart<'_>,
    map: &SlotMap,
    stats: &mut ExtruderRemapStats,
) -> Result<(Event<'static>, Option<u8>)> {
    rewrite_extruder_element(e, map, stats, false)
}

fn maybe_rewrite_extruder_metadata_empty(
    e: BytesStart<'_>,
    map: &SlotMap,
    stats: &mut ExtruderRemapStats,
) -> Result<(Event<'static>, Option<u8>)> {
    rewrite_extruder_element(e, map, stats, true)
}

fn rewrite_extruder_element(
    e: BytesStart<'_>,
    map: &SlotMap,
    stats: &mut ExtruderRemapStats,
    empty: bool,
) -> Result<(Event<'static>, Option<u8>)> {
    let mut key: Option<String> = None;
    let mut value: Option<String> = None;
    let mut other_attrs: Vec<(String, String)> = Vec::new();

    for attr_result in e.attributes() {
        let attr = attr_result
            .map_err(|err| Error::msg(format!("model_settings attribute parse failed: {err}")))?;
        let an = local_name(attr.key.as_ref());
        let key_str = qname_to_str(attr.key.as_ref())?;
        let av = attr
            .normalized_value(XmlVersion::Implicit1_0)
            .map_err(|err| Error::msg(format!("model_settings attribute normalize failed: {err}")))?
            .into_owned();
        if an == b"key" {
            key = Some(av);
        } else if an == b"value" {
            value = Some(av);
        } else {
            other_attrs.push((key_str, av));
        }
    }

    if key.as_deref() != Some("extruder") {
        // Pass through unchanged.
        let owned = e.into_owned();
        let event = if empty {
            Event::Empty(owned)
        } else {
            Event::Start(owned)
        };
        return Ok((event, None));
    }

    let src_str = value.unwrap_or_default();
    let src_slot: u8 = src_str.parse().map_err(|_| {
        Error::msg(format!(
            "model_settings extruder value is not a u8: '{src_str}'"
        ))
    })?;
    if src_slot < 1 {
        return Err(Error::msg(format!(
            "model_settings extruder value must be >= 1, got {src_slot}"
        )));
    }
    let dest = map.map_slot(src_slot);
    if dest != src_slot {
        stats.rewritten += 1;
    }
    *stats.histogram_out.entry(dest).or_insert(0) += 1;

    let name_str = qname_to_str(e.name().as_ref())?;
    let mut new_elem = BytesStart::new(name_str);
    // Preserve attribute order: key then value then others (typical Bambu order).
    new_elem.push_attribute(("key", "extruder"));
    let dest_s = dest.to_string();
    new_elem.push_attribute(("value", dest_s.as_str()));
    for (k, v) in &other_attrs {
        new_elem.push_attribute((k.as_str(), v.as_str()));
    }

    let event = if empty {
        Event::Empty(new_elem)
    } else {
        Event::Start(new_elem)
    };
    Ok((event, Some(dest)))
}

fn write_extruder_metadata<W: std::io::Write>(writer: &mut Writer<W>, dest: u8) -> Result<()> {
    let mut elem = BytesStart::new("metadata");
    elem.push_attribute(("key", "extruder"));
    let dest_s = dest.to_string();
    elem.push_attribute(("value", dest_s.as_str()));
    writer
        .write_event(Event::Empty(elem))
        .map_err(|err| Error::msg(format!("model_settings write failed: {err}")))?;
    Ok(())
}

fn local_name(qname: &[u8]) -> &[u8] {
    qname.rsplit(|b| *b == b':').next().unwrap_or(qname)
}

/// Decode an XML qname to UTF-8. Invalid UTF-8 is a hard error (DoD-7 hygiene).
fn qname_to_str(qname: &[u8]) -> Result<String> {
    String::from_utf8(qname.to_vec())
        .map_err(|_| Error::msg("model_settings: invalid UTF-8 in XML element/attribute qname"))
}

/// Validate Start/Empty element name and all attribute keys as UTF-8 (DoD-7).
fn validate_start_qnames(e: &BytesStart<'_>) -> Result<()> {
    qname_to_str(e.name().as_ref())?;
    for attr_result in e.attributes() {
        let attr = attr_result
            .map_err(|err| Error::msg(format!("model_settings attribute parse failed: {err}")))?;
        qname_to_str(attr.key.as_ref())?;
    }
    Ok(())
}

/// Validate End element name as UTF-8 (DoD-7).
fn validate_end_qname(e: &BytesEnd<'_>) -> Result<()> {
    qname_to_str(e.name().as_ref())?;
    Ok(())
}

/// Count `paint_color` attribute occurrences in model XML (root or nested).
///
/// Lightweight scan: count occurrences of the attribute name token in the file.
/// Good enough for presence detection (dumpling has 0).
pub fn count_paint_color_attrs(bytes: &[u8]) -> u32 {
    // Avoid full DOM; search for paint_color= in a case-sensitive way matching Bambu/Orca.
    let needle = b"paint_color=";
    let mut count = 0u32;
    let mut i = 0;
    while i + needle.len() <= bytes.len() {
        if &bytes[i..i + needle.len()] == needle {
            count += 1;
            i += needle.len();
        } else {
            i += 1;
        }
    }
    count
}

/// Known `subtype` values on `<part>` (Bambu/Orca). Unknown subtypes warn only.
const KNOWN_PART_SUBTYPES: &[&str] = &[
    "normal_part",
    "negative_part",
    "modifier",
    "parameter_modifier",
    "support_enforcer",
    "support_blocker",
];

/// Scan model_settings for unknown `subtype` attribute values (warn-only allowlist).
///
/// Returns sorted unique unknown subtype strings. Never fails convert by itself —
/// XML parse errors still return `Err`.
pub fn collect_unknown_subtypes(bytes: &[u8]) -> Result<Vec<String>> {
    let mut reader = Reader::from_reader(Cursor::new(bytes));
    reader.config_mut().trim_text(true);
    let mut unknown: BTreeMap<String, ()> = BTreeMap::new();
    let mut buf = Vec::new();

    loop {
        match reader.read_event_into(&mut buf) {
            Ok(Event::Start(e)) | Ok(Event::Empty(e)) => {
                let local = local_name(e.name().as_ref()).to_vec();
                if local == b"part" {
                    for attr_result in e.attributes() {
                        let attr = attr_result.map_err(|err| {
                            Error::msg(format!("model_settings attribute parse failed: {err}"))
                        })?;
                        if local_name(attr.key.as_ref()) == b"subtype" {
                            let v = attr
                                .normalized_value(XmlVersion::Implicit1_0)
                                .map_err(|err| {
                                    Error::msg(format!(
                                        "model_settings attribute normalize failed: {err}"
                                    ))
                                })?
                                .into_owned();
                            if !v.is_empty() && !KNOWN_PART_SUBTYPES.contains(&v.as_str()) {
                                unknown.insert(v, ());
                            }
                        }
                    }
                }
            }
            Ok(Event::Eof) => break,
            Err(e) => return Err(Error::xml("model_settings subtype scan", e)),
            _ => {}
        }
        buf.clear();
    }

    Ok(unknown.into_keys().collect())
}

/// Object ids found in a 3MF root model (resources/build).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ModelObjectIds {
    /// `<object id="…">` in resources (order of appearance).
    pub resource_object_ids: Vec<u32>,
    /// `<item objectid="…">` in build (order of appearance).
    pub build_object_ids: Vec<u32>,
}

impl ModelObjectIds {
    /// Prefer build items; fall back to resource object ids. Deduped, stable order.
    pub fn printable_ids(&self) -> Vec<u32> {
        let mut out = if self.build_object_ids.is_empty() {
            self.resource_object_ids.clone()
        } else {
            self.build_object_ids.clone()
        };
        let mut seen = std::collections::BTreeSet::new();
        out.retain(|id| seen.insert(*id));
        out
    }
}

/// Parse object / build item ids from a root `3D/3dmodel.model` (streaming, no DOM).
pub fn parse_model_object_ids(model_xml: &[u8]) -> Result<ModelObjectIds> {
    let mut reader = Reader::from_reader(Cursor::new(model_xml));
    reader.config_mut().trim_text(true);
    let mut ids = ModelObjectIds::default();
    let mut buf = Vec::new();

    loop {
        match reader.read_event_into(&mut buf) {
            Ok(Event::Start(e)) | Ok(Event::Empty(e)) => {
                let local = local_name(e.name().as_ref()).to_vec();
                if local == b"object"
                    && let Some(id) = attr_u32(&e, b"id")?
                {
                    ids.resource_object_ids.push(id);
                } else if local == b"item"
                    && let Some(id) = attr_u32(&e, b"objectid")?
                {
                    ids.build_object_ids.push(id);
                }
            }
            Ok(Event::Eof) => break,
            Err(e) => return Err(Error::xml("3dmodel.model object ids", e)),
            _ => {}
        }
        buf.clear();
    }
    Ok(ids)
}

fn attr_u32(e: &BytesStart<'_>, key: &[u8]) -> Result<Option<u32>> {
    for attr_result in e.attributes() {
        let attr = attr_result
            .map_err(|err| Error::msg(format!("model attribute parse failed: {err}")))?;
        if local_name(attr.key.as_ref()) == key {
            let v = attr
                .normalized_value(XmlVersion::Implicit1_0)
                .map_err(|err| Error::msg(format!("model attribute normalize failed: {err}")))?
                .into_owned();
            let n: u32 = v
                .parse()
                .map_err(|_| Error::msg(format!("model id is not a u32: '{v}'")))?;
            return Ok(Some(n));
        }
    }
    Ok(None)
}

/// Synthesize minimal Bambu/Orca-style `model_settings.config` for S2 (**C4**).
///
/// One plate (plater_id=1) plus an `<object>` / default extruder per printable id.
/// Extruder defaults to `map.map_slot(1)` (usually 1).
pub fn synthesize_model_settings(object_ids: &[u32], map: &SlotMap) -> Vec<u8> {
    let dest = map.map_slot(1).max(1);
    let mut out = String::from(
        r#"<?xml version="1.0" encoding="UTF-8"?>
<config>
"#,
    );
    let ids: Vec<u32> = if object_ids.is_empty() {
        vec![1]
    } else {
        object_ids.to_vec()
    };
    for id in &ids {
        out.push_str(&format!(
            r#"  <object id="{id}">
    <metadata key="name" value="object_{id}"/>
    <metadata key="extruder" value="{dest}"/>
    <part id="1" subtype="normal_part">
      <metadata key="name" value="part_1"/>
      <metadata key="extruder" value="{dest}"/>
    </part>
  </object>
"#
        ));
    }
    out.push_str(
        r#"  <plate>
    <metadata key="plater_id" value="1"/>
    <metadata key="plater_name" value="Plate 1"/>
    <metadata key="locked" value="false"/>
"#,
    );
    for (i, id) in ids.iter().enumerate() {
        out.push_str(&format!(
            r#"    <model_instance>
      <metadata key="object_id" value="{id}"/>
      <metadata key="instance_id" value="0"/>
      <metadata key="identify_id" value="{}"/>
    </model_instance>
"#,
            i + 1
        ));
    }
    out.push_str("  </plate>\n  <assemble>\n");
    for id in &ids {
        out.push_str(&format!(
            r#"    <assemble_item object_id="{id}" instance_id="0" transform="1 0 0 0 1 0 0 0 1 0 0 0" offset="0 0 0"/>
"#
        ));
    }
    out.push_str("  </assemble>\n</config>\n");
    out.into_bytes()
}

#[cfg(test)]
#[allow(non_snake_case)] // track test naming: feature__condition__expected
mod tests {
    use super::*;

    #[test]
    fn parse_model_settings__plates_and_extruders__histogram() {
        let xml = br#"<?xml version="1.0"?>
<config>
  <object id="1">
    <metadata key="extruder" value="1"/>
    <part id="1">
      <metadata key="extruder" value="2"/>
    </part>
    <part id="2">
      <metadata key="extruder" value="2"/>
    </part>
  </object>
  <plate><metadata key="plater_id" value="1"/></plate>
  <plate><metadata key="plater_id" value="2"/></plate>
</config>
"#;
        let s = parse_model_settings(xml).expect("parse");
        assert_eq!(s.plate_count, 2);
        assert_eq!(s.extruder_histogram.get(&1), Some(&1));
        assert_eq!(s.extruder_histogram.get(&2), Some(&2));
    }

    #[test]
    fn count_paint_color_attrs__present_and_absent() {
        assert_eq!(count_paint_color_attrs(b"<triangle paint_color=\"4\"/>"), 1);
        assert_eq!(count_paint_color_attrs(b"<triangle v1=\"0\"/>"), 0);
    }

    #[test]
    fn parse_model_settings__unclosed_xml__returns_error() {
        let xml = br#"<?xml version="1.0"?><config><plate><metadata key="extruder" value="1""#;
        let err = parse_model_settings(xml).expect_err("must fail");
        assert!(
            matches!(err, Error::Xml { .. }),
            "expected XML error, got {err:?}"
        );
    }

    #[test]
    fn parse_model_settings__malformed_attribute__returns_error() {
        // Missing `=` after attribute name — quick-xml yields AttrError (not flatten-skip).
        let xml = br#"<?xml version="1.0"?><config><metadata key extruder value="1"/></config>"#;
        let err = parse_model_settings(xml).expect_err("must fail");
        let msg = err.to_string();
        assert!(
            msg.contains("attribute") || matches!(err, Error::Xml { .. } | Error::Message(_)),
            "expected attribute/XML error, got {err:?}"
        );
    }

    #[test]
    fn remap_extruders__swap() {
        let xml = br#"<?xml version="1.0"?>
<config>
  <object id="1">
    <metadata key="extruder" value="1"/>
    <part id="1" subtype="normal_part">
      <metadata key="extruder" value="2"/>
    </part>
    <part id="2" subtype="normal_part">
      <metadata key="extruder" value="2"/>
    </part>
  </object>
</config>
"#;
        let map = SlotMap::from_pairs([(1, 2), (2, 1)]);
        let (out, stats) = remap_model_settings_extruders(xml, &map).expect("remap");
        assert_eq!(stats.rewritten, 3);
        // After swap: one 2 (from object 1), two 1s (from parts 2)
        assert_eq!(stats.histogram_out.get(&1), Some(&2));
        assert_eq!(stats.histogram_out.get(&2), Some(&1));

        let parsed = parse_model_settings(&out).expect("reparse");
        assert_eq!(parsed.extruder_histogram.get(&1), Some(&2));
        assert_eq!(parsed.extruder_histogram.get(&2), Some(&1));
    }

    #[test]
    fn remap_extruders__missing_defaults_to_1__inserts_when_mapped() {
        let xml = br#"<?xml version="1.0"?>
<config>
  <object id="1">
    <part id="1" subtype="normal_part">
      <metadata key="name" value="Body"/>
    </part>
  </object>
</config>
"#;
        let map = SlotMap::from_pairs([(1, 3)]);
        let (out, stats) = remap_model_settings_extruders(xml, &map).expect("remap");
        // Object missing extruder + part missing → inject both if map(1)=3
        assert!(stats.injected >= 1, "expected inject, stats={stats:?}");
        let text = String::from_utf8(out.clone()).expect("utf8");
        assert!(
            text.contains("key=\"extruder\"") && text.contains("value=\"3\""),
            "expected injected extruder=3, got: {text}"
        );
        let parsed = parse_model_settings(&out).expect("reparse");
        assert!(
            parsed.extruder_histogram.get(&3).copied().unwrap_or(0) >= 1,
            "histogram={:?}",
            parsed.extruder_histogram
        );
    }

    #[test]
    fn remap_extruders__negative_part__no_inject() {
        let xml = br#"<?xml version="1.0"?>
<config>
  <object id="1">
    <metadata key="extruder" value="1"/>
    <part id="1" subtype="negative_part">
      <metadata key="name" value="Cut"/>
    </part>
  </object>
</config>
"#;
        let map = SlotMap::from_pairs([(1, 2)]);
        let (out, stats) = remap_model_settings_extruders(xml, &map).expect("remap");
        // Object extruder rewritten; negative_part should not get inject
        assert_eq!(stats.injected, 0);
        let parsed = parse_model_settings(&out).expect("reparse");
        // Only the object extruder (1→2)
        assert_eq!(parsed.extruder_histogram.get(&2), Some(&1));
        assert!(!parsed.extruder_histogram.contains_key(&1));
    }

    #[test]
    fn model_settings__invalid_utf8_qname__errors() {
        // Helper still rejects non-UTF-8 qname bytes directly.
        let bad = b"el\xFF";
        let err = super::qname_to_str(bad).expect_err("must fail");
        assert!(
            err.to_string().contains("UTF-8") || err.to_string().contains("qname"),
            "got {err}"
        );
    }

    #[test]
    fn parse_model_settings__invalid_utf8_element_qname__errors() {
        // quick-xml accepts raw 0xFF in element names; DoD-7 requires Error on parse.
        let xml = b"<?xml version=\"1.0\"?><config><el\xFFname/></config>";
        let err = parse_model_settings(xml).expect_err("must fail on invalid element qname");
        assert!(
            err.to_string().contains("UTF-8") || err.to_string().contains("qname"),
            "got {err}"
        );
    }

    #[test]
    fn parse_model_settings__invalid_utf8_attr_qname__errors() {
        let xml =
            b"<?xml version=\"1.0\"?><config><metadata ke\xFFy=\"extruder\" value=\"1\"/></config>";
        let err = parse_model_settings(xml).expect_err("must fail on invalid attr qname");
        assert!(
            err.to_string().contains("UTF-8") || err.to_string().contains("qname"),
            "got {err}"
        );
    }

    #[test]
    fn parse_model_settings__invalid_utf8_end_qname__errors() {
        // Start name is valid UTF-8; End carries invalid UTF-8 — still Error (DoD-7).
        // quick-xml returns End with the closing tag's raw name bytes.
        let xml = b"<?xml version=\"1.0\"?><config><plate></pl\xFFate></config>";
        let err = parse_model_settings(xml).expect_err("must fail on invalid end qname");
        // May surface as UTF-8 qname Error or XML mismatch depending on parser checks.
        let msg = err.to_string();
        assert!(
            msg.contains("UTF-8")
                || msg.contains("qname")
                || matches!(err, Error::Xml { .. } | Error::Message(_)),
            "got {err}"
        );
    }

    #[test]
    fn remap_model_settings__invalid_utf8_element_qname__errors() {
        // Rewrite path still rejects invalid qnames when End/metadata rewrite runs.
        let xml = b"<?xml version=\"1.0\"?><config><object id=\"1\"><part id=\"1\" subtype=\"normal_part\"></par\xFFt></object></config>";
        let map = SlotMap::from_pairs([(1, 2)]);
        let err = remap_model_settings_extruders(xml, &map).expect_err("must fail");
        let msg = err.to_string();
        assert!(
            msg.contains("UTF-8")
                || msg.contains("qname")
                || matches!(err, Error::Xml { .. } | Error::Message(_)),
            "got {err}"
        );
    }

    #[test]
    fn collect_unknown_subtypes__warns_on_unknown() {
        let xml = br#"<?xml version="1.0"?>
<config>
  <object id="1">
    <part id="1" subtype="normal_part"/>
    <part id="2" subtype="fancy_bambu_only"/>
  </object>
</config>
"#;
        let unknown = collect_unknown_subtypes(xml).expect("scan");
        assert_eq!(unknown, vec!["fancy_bambu_only".to_string()]);
    }

    #[test]
    fn synthesize_model_settings__references_object_ids() {
        let bytes = synthesize_model_settings(&[7, 9], &SlotMap::identity());
        let text = String::from_utf8(bytes).expect("utf8");
        assert!(text.contains("id=\"7\""));
        assert!(text.contains("id=\"9\""));
        assert!(text.contains("plater_id\" value=\"1\"") || text.contains("value=\"1\""));
        assert!(text.contains("object_id\" value=\"7\"") || text.contains("object_id"));
        let summary = parse_model_settings(text.as_bytes()).expect("parse");
        assert_eq!(summary.plate_count, 1);
        assert!(!summary.extruder_histogram.is_empty());
    }

    #[test]
    fn parse_model_object_ids__resources_and_build() {
        let xml = br#"<?xml version="1.0"?>
<model>
 <resources>
  <object id="1" type="model"/>
  <object id="2" type="model"/>
 </resources>
 <build>
  <item objectid="2"/>
  <item objectid="1"/>
 </build>
</model>
"#;
        let ids = parse_model_object_ids(xml).expect("parse");
        assert_eq!(ids.resource_object_ids, vec![1, 2]);
        assert_eq!(ids.build_object_ids, vec![2, 1]);
        assert_eq!(ids.printable_ids(), vec![2, 1]);
    }
}
