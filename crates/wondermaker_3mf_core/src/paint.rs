//! Bambu/Orca triangle `paint_color` encode/decode and model XML rewrite.
//!
//! Paint codes are proprietary (not ISO 3MF). Algorithm:
//! 1. Decode attribute string → slot numbers (longest-match L→R or high→low strip
//!    on a **copy of the original only**)
//! 2. Map each slot via [`SlotMap::map_slot`] (numeric domain)
//! 3. Encode once — **never** multi-pass code string replace (constraint C1)

use std::io::Cursor;

use quick_xml::Reader;
use quick_xml::Writer;
use quick_xml::XmlVersion;
use quick_xml::events::{BytesEnd, BytesStart, BytesText, Event};

use crate::error::{Error, Result};
use crate::slot_map::SlotMap;

/// Slot codes for 1-based slots 1..=16 (Printago SLOT_CODES order).
///
/// Index 0 = slot 1 (`4`), index 15 = slot 16 (`DC`).
pub const SLOT_CODES: [&str; 16] = [
    "4", "8", "0C", "1C", "2C", "3C", "4C", "5C", "6C", "7C", "8C", "9C", "AC", "BC", "CC", "DC",
];

/// Stats from remapping paint attributes in a model file.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct PaintRemapStats {
    pub attrs_seen: u32,
    pub attrs_rewritten: u32,
    /// Non-empty residual after decode (unknown tokens).
    pub residual_warnings: Vec<String>,
}

/// Decode a `paint_color` attribute value into 1-based slot numbers.
///
/// Uses high→low strip on a **working copy of the original only** (mask on original
/// bytes) so longer codes (`8C`, `DC`, …) win over shorter prefixes (`8`, `4`).
/// Slot order follows left-to-right appearance in the original string.
pub fn decode_paint_color(s: &str) -> (Vec<u8>, String) {
    decode_paint_color_high_to_low(s)
}

/// Left-to-right longest-match tokenizer (2-char codes before 1-char).
///
/// Alternate algorithm (plan allows LTR or high→low). Kept under `cfg(test)` for
/// equivalence checks against [`decode_paint_color_high_to_low`].
#[cfg(test)]
fn decode_paint_color_ltr(s: &str) -> (Vec<u8>, String) {
    let bytes = s.as_bytes();
    let mut slots = Vec::new();
    let mut residual = String::new();
    let mut i = 0usize;

    while i < bytes.len() {
        // Try 2-char codes first (slots with C-suffix and letter codes).
        let mut matched = false;
        if i + 2 <= bytes.len() {
            let two = std::str::from_utf8(&bytes[i..i + 2]).unwrap_or("");
            if let Some(slot) = code_to_slot(two) {
                slots.push(slot);
                i += 2;
                matched = true;
            }
        }
        if !matched && i < bytes.len() {
            let one = std::str::from_utf8(&bytes[i..i + 1]).unwrap_or("");
            if let Some(slot) = code_to_slot(one) {
                slots.push(slot);
                i += 1;
                matched = true;
            }
        }
        if !matched {
            // Unknown char — keep as residual and advance one byte.
            residual.push(bytes[i] as char);
            i += 1;
        }
    }
    (slots, residual)
}

/// High→low strip decode that preserves original order via recorded offsets.
///
/// Exposed for tests that want the Printago strip algorithm explicitly.
pub fn decode_paint_color_high_to_low(s: &str) -> (Vec<u8>, String) {
    let original = s.as_bytes();
    let mut covered = vec![false; original.len()];
    let mut hits: Vec<(usize, u8)> = Vec::new();

    for slot in (1u8..=16).rev() {
        let code = SLOT_CODES[(slot as usize) - 1].as_bytes();
        let mut start = 0usize;
        while start + code.len() <= original.len() {
            // Skip if any byte already covered
            if covered[start..start + code.len()].iter().any(|&c| c) {
                start += 1;
                continue;
            }
            if &original[start..start + code.len()] == code {
                for c in &mut covered[start..start + code.len()] {
                    *c = true;
                }
                hits.push((start, slot));
                start += code.len();
            } else {
                start += 1;
            }
        }
    }

    hits.sort_by_key(|(pos, _)| *pos);
    let slots: Vec<u8> = hits.into_iter().map(|(_, s)| s).collect();
    let mut residual = String::new();
    for (i, &b) in original.iter().enumerate() {
        if !covered[i] {
            residual.push(b as char);
        }
    }
    (slots, residual)
}

/// Map a single SLOT_CODES token to a 1-based slot (LTR decoder only).
#[cfg(test)]
fn code_to_slot(code: &str) -> Option<u8> {
    SLOT_CODES
        .iter()
        .position(|&c| c == code)
        .map(|i| (i + 1) as u8)
}

/// Encode 1-based slot numbers into a `paint_color` attribute string.
pub fn encode_paint_color(slots: &[u8]) -> Result<String> {
    let mut out = String::new();
    for &slot in slots {
        if !(1..=16).contains(&slot) {
            return Err(Error::msg(format!(
                "cannot encode paint slot {slot} (expected 1..=16)"
            )));
        }
        out.push_str(SLOT_CODES[(slot as usize) - 1]);
    }
    Ok(out)
}

/// Decode → map_slot (numeric) → encode once. Never multi-pass string replace.
pub fn remap_paint_color(s: &str, map: &SlotMap) -> Result<(String, Option<String>)> {
    let (slots, residual) = decode_paint_color(s);
    let mapped: Vec<u8> = slots.iter().map(|&src| map.map_slot(src)).collect();
    // Dest after map must be encodable; ZR uses 1..=4 but codes exist to 16.
    let encoded = encode_paint_color(&mapped)?;
    let residual_warn = if residual.is_empty() {
        None
    } else {
        Some(format!(
            "paint_color residual after decode (unknown tokens): '{residual}' in '{s}'"
        ))
    };
    Ok((encoded, residual_warn))
}

/// True if the buffer contains a `paint_color=` attribute (fast probe).
pub fn has_paint_color_attr(bytes: &[u8]) -> bool {
    memmem_contains(bytes, b"paint_color=")
}

fn memmem_contains(haystack: &[u8], needle: &[u8]) -> bool {
    if needle.is_empty() || haystack.len() < needle.len() {
        return false;
    }
    haystack.windows(needle.len()).any(|w| w == needle)
}

/// Rewrite all `triangle` `paint_color` attributes via quick-xml event stream.
///
/// Returns `Ok(None)` when no `paint_color=` is present (caller may raw_copy).
pub fn remap_model_paint(xml: &[u8], map: &SlotMap) -> Result<Option<(Vec<u8>, PaintRemapStats)>> {
    if !has_paint_color_attr(xml) {
        return Ok(None);
    }

    let mut reader = Reader::from_reader(Cursor::new(xml));
    reader.config_mut().trim_text(false);
    // Preserve original whitespace/structure as much as possible.
    let mut writer = Writer::new(Cursor::new(Vec::with_capacity(xml.len())));
    let mut stats = PaintRemapStats::default();
    let mut buf = Vec::new();

    loop {
        match reader.read_event_into(&mut buf) {
            Ok(Event::Start(e)) => {
                let event = if local_name(e.name().as_ref()) == b"triangle" {
                    rewrite_triangle_start(e, map, &mut stats)?
                } else {
                    Event::Start(e.into_owned())
                };
                writer
                    .write_event(event)
                    .map_err(|err| Error::msg(format!("paint model write failed: {err}")))?;
            }
            Ok(Event::Empty(e)) => {
                let event = if local_name(e.name().as_ref()) == b"triangle" {
                    rewrite_triangle_empty(e, map, &mut stats)?
                } else {
                    Event::Empty(e.into_owned())
                };
                writer
                    .write_event(event)
                    .map_err(|err| Error::msg(format!("paint model write failed: {err}")))?;
            }
            Ok(Event::End(e)) => {
                let name_bytes = e.name().as_ref().to_vec();
                let name_str = std::str::from_utf8(&name_bytes).map_err(|err| {
                    Error::msg(format!(
                        "paint model end-tag qname is not valid UTF-8: {err}"
                    ))
                })?;
                writer
                    .write_event(Event::End(BytesEnd::new(name_str)))
                    .map_err(|err| Error::msg(format!("paint model write failed: {err}")))?;
            }
            Ok(Event::Text(t)) => {
                // Preserve text as-is
                let content = t
                    .decode()
                    .map_err(|err| Error::msg(format!("paint model text decode failed: {err}")))?;
                writer
                    .write_event(Event::Text(BytesText::new(&content)))
                    .map_err(|err| Error::msg(format!("paint model write failed: {err}")))?;
            }
            Ok(Event::CData(c)) => {
                writer
                    .write_event(Event::CData(c.into_owned()))
                    .map_err(|err| Error::msg(format!("paint model write failed: {err}")))?;
            }
            Ok(Event::Comment(c)) => {
                writer
                    .write_event(Event::Comment(c.into_owned()))
                    .map_err(|err| Error::msg(format!("paint model write failed: {err}")))?;
            }
            Ok(Event::Decl(d)) => {
                writer
                    .write_event(Event::Decl(d.into_owned()))
                    .map_err(|err| Error::msg(format!("paint model write failed: {err}")))?;
            }
            Ok(Event::PI(p)) => {
                writer
                    .write_event(Event::PI(p.into_owned()))
                    .map_err(|err| Error::msg(format!("paint model write failed: {err}")))?;
            }
            Ok(Event::DocType(d)) => {
                writer
                    .write_event(Event::DocType(d.into_owned()))
                    .map_err(|err| Error::msg(format!("paint model write failed: {err}")))?;
            }
            Ok(Event::GeneralRef(g)) => {
                writer
                    .write_event(Event::GeneralRef(g.into_owned()))
                    .map_err(|err| Error::msg(format!("paint model write failed: {err}")))?;
            }
            Ok(Event::Eof) => break,
            Err(e) => return Err(Error::xml("model paint remap", e)),
        }
        buf.clear();
    }

    let out = writer.into_inner().into_inner();
    Ok(Some((out, stats)))
}

fn rewrite_triangle_start(
    e: BytesStart<'_>,
    map: &SlotMap,
    stats: &mut PaintRemapStats,
) -> Result<Event<'static>> {
    let new_start = rewrite_triangle_attrs(e, map, stats)?;
    Ok(Event::Start(new_start))
}

fn rewrite_triangle_empty(
    e: BytesStart<'_>,
    map: &SlotMap,
    stats: &mut PaintRemapStats,
) -> Result<Event<'static>> {
    let new_start = rewrite_triangle_attrs(e, map, stats)?;
    Ok(Event::Empty(new_start))
}

fn rewrite_triangle_attrs(
    e: BytesStart<'_>,
    map: &SlotMap,
    stats: &mut PaintRemapStats,
) -> Result<BytesStart<'static>> {
    let name_bytes = e.name().as_ref().to_vec();
    let name_str = String::from_utf8_lossy(&name_bytes).into_owned();
    let mut new_elem = BytesStart::new(name_str);

    for attr_result in e.attributes() {
        let attr = attr_result
            .map_err(|err| Error::msg(format!("paint triangle attribute parse failed: {err}")))?;
        let key_bytes = attr.key.as_ref().to_vec();
        let key_str = String::from_utf8_lossy(&key_bytes).into_owned();
        let value = attr
            .normalized_value(XmlVersion::Implicit1_0)
            .map_err(|err| Error::msg(format!("paint triangle attribute normalize failed: {err}")))?
            .into_owned();

        if local_name(&key_bytes) == b"paint_color" {
            stats.attrs_seen += 1;
            let (remapped, residual) = remap_paint_color(&value, map)?;
            if let Some(w) = residual {
                stats.residual_warnings.push(w);
            }
            if remapped != value {
                stats.attrs_rewritten += 1;
            }
            new_elem.push_attribute((key_str.as_str(), remapped.as_str()));
        } else {
            new_elem.push_attribute((key_str.as_str(), value.as_str()));
        }
    }

    Ok(new_elem)
}

fn local_name(qname: &[u8]) -> &[u8] {
    qname.rsplit(|b| *b == b':').next().unwrap_or(qname)
}

/// Collect all source slots used in paint attributes (for ZR validation).
pub fn collect_paint_source_slots(xml: &[u8]) -> Result<Vec<u8>> {
    if !has_paint_color_attr(xml) {
        return Ok(Vec::new());
    }
    let mut reader = Reader::from_reader(Cursor::new(xml));
    reader.config_mut().trim_text(true);
    let mut slots = Vec::new();
    let mut buf = Vec::new();

    loop {
        match reader.read_event_into(&mut buf) {
            Ok(Event::Start(e)) | Ok(Event::Empty(e)) => {
                if local_name(e.name().as_ref()) == b"triangle" {
                    for attr_result in e.attributes() {
                        let attr = attr_result.map_err(|err| {
                            Error::msg(format!("paint collect attribute parse failed: {err}"))
                        })?;
                        if local_name(attr.key.as_ref()) == b"paint_color" {
                            let value = attr
                                .normalized_value(XmlVersion::Implicit1_0)
                                .map_err(|err| {
                                    Error::msg(format!(
                                        "paint collect attribute normalize failed: {err}"
                                    ))
                                })?
                                .into_owned();
                            let (decoded, _) = decode_paint_color(&value);
                            slots.extend(decoded);
                        }
                    }
                }
            }
            Ok(Event::Eof) => break,
            Err(e) => return Err(Error::xml("collect paint slots", e)),
            _ => {}
        }
        buf.clear();
    }
    slots.sort_unstable();
    slots.dedup();
    Ok(slots)
}

#[cfg(test)]
#[allow(non_snake_case)] // track test naming: feature__condition__expected
mod tests {
    use super::*;

    #[test]
    fn paint_roundtrip__all_codes_1_to_16() {
        for slot in 1u8..=16 {
            let code = SLOT_CODES[(slot as usize) - 1];
            let (decoded, residual) = decode_paint_color(code);
            assert!(
                residual.is_empty(),
                "residual for slot {slot} code {code}: {residual}"
            );
            assert_eq!(decoded, vec![slot], "decode slot {slot}");
            let encoded = encode_paint_color(&decoded).expect("encode");
            assert_eq!(encoded, code, "roundtrip slot {slot}");
        }
    }

    #[test]
    fn paint_partial_match__8_vs_8C() {
        let (d, r) = decode_paint_color("8C");
        assert!(r.is_empty());
        assert_eq!(d, vec![11], "8C must be slot 11, not 2+something");

        let (d2, r2) = decode_paint_color("8");
        assert!(r2.is_empty());
        assert_eq!(d2, vec![2]);

        // Combined: slot2 then slot11
        let (d3, r3) = decode_paint_color("88C");
        assert!(r3.is_empty());
        assert_eq!(d3, vec![2, 11]);
    }

    #[test]
    fn paint_remap__1_to_4() {
        let map = SlotMap::from_pairs([(1, 4)]);
        let (out, warn) = remap_paint_color("4", &map).expect("remap");
        assert!(warn.is_none());
        assert_eq!(out, "1C"); // slot 4 code
    }

    #[test]
    fn paint_remap__no_double_replace__dc_to_1c_and_1c_to_4() {
        // Map 16→4 and 4→1. Code DC (16) must become 1C (4), not then get re-mapped to 4.
        let map = SlotMap::from_pairs([(16, 4), (4, 1)]);
        let (out, warn) = remap_paint_color("DC", &map).expect("remap DC");
        assert!(warn.is_none());
        assert_eq!(
            out, "1C",
            "DC→slot16→map→4→encode 1C once (not further to 4)"
        );

        // Multi-code: DC then 1C → slots 16,4 → mapped 4,1 → "1C"+"4"
        let (out2, warn2) = remap_paint_color("DC1C", &map).expect("remap multi");
        assert!(warn2.is_none());
        assert_eq!(out2, "1C4");

        // Pure 1C (slot 4) → slot 1 → "4"
        let (out3, _) = remap_paint_color("1C", &map).expect("remap 1C");
        assert_eq!(out3, "4");
    }

    #[test]
    fn paint_high_to_low_matches_ltr_on_valid() {
        // Real equivalence: LTR longest-match vs high→low strip (not self-comparison).
        for sample in ["4", "8", "8C", "DC1C4", "0C2C"] {
            let (ltr_slots, ltr_res) = decode_paint_color_ltr(sample);
            let (htl_slots, htl_res) = decode_paint_color_high_to_low(sample);
            assert_eq!(ltr_slots, htl_slots, "slots for {sample}");
            assert_eq!(ltr_res, htl_res, "residual for {sample}");
            // Public decode path uses high→low.
            let (pub_slots, pub_res) = decode_paint_color(sample);
            assert_eq!(pub_slots, htl_slots, "public decode for {sample}");
            assert_eq!(pub_res, htl_res, "public residual for {sample}");
        }
    }

    #[test]
    fn remap_model_paint__triangle_attrs() {
        let xml = br#"<?xml version="1.0"?>
<model>
 <triangles>
  <triangle v1="0" v2="1" v3="2" paint_color="4"/>
  <triangle v1="0" v2="1" v3="2" paint_color="DC"/>
 </triangles>
</model>
"#;
        let map = SlotMap::from_pairs([(1, 4), (16, 4), (4, 1)]);
        let (out, stats) = remap_model_paint(xml, &map)
            .expect("remap")
            .expect("has paint");
        assert_eq!(stats.attrs_seen, 2);
        assert!(stats.attrs_rewritten >= 1);
        let text = String::from_utf8(out).expect("utf8");
        assert!(text.contains("paint_color=\"1C\""), "got: {text}");
        // Second attr: DC → 1C
        assert_eq!(text.matches("paint_color=\"1C\"").count(), 2);
    }

    #[test]
    fn remap_model_paint__no_paint__returns_none() {
        let xml = br#"<?xml version="1.0"?><model><triangle v1="0" v2="1" v3="2"/></model>"#;
        let map = SlotMap::from_pairs([(1, 2)]);
        assert!(remap_model_paint(xml, &map).expect("ok").is_none());
    }
}
