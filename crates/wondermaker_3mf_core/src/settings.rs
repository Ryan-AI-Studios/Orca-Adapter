//! `project_settings.config` JSON helpers (preserve_order).

use serde_json::{Value, json};

use crate::error::{Error, Result};
use crate::slot_map::SlotMap;

/// Parse project_settings JSON bytes into an ordered `Value`.
pub fn parse_project_settings(bytes: &[u8]) -> Result<Value> {
    serde_json::from_slice(bytes).map_err(|e| Error::json("project_settings.config", e))
}

/// Serialize project_settings with key order preserved (pretty + trailing newline).
pub fn serialize_project_settings(value: &Value) -> Result<Vec<u8>> {
    let mut out = serde_json::to_vec_pretty(value)
        .map_err(|e| Error::json("serialize project_settings", e))?;
    if !out.ends_with(b"\n") {
        out.push(b'\n');
    }
    Ok(out)
}

/// String field from project_settings (if present and a string).
pub fn string_field(settings: &Value, key: &str) -> Option<String> {
    settings
        .get(key)
        .and_then(|v| v.as_str())
        .map(str::to_owned)
}

/// Extract a JSON array of strings (or stringify non-strings).
pub fn string_array_field(settings: &Value, key: &str) -> Vec<String> {
    match settings.get(key) {
        Some(Value::Array(arr)) => arr
            .iter()
            .map(|v| match v {
                Value::String(s) => s.clone(),
                other => other.to_string(),
            })
            .collect(),
        Some(Value::String(s)) => vec![s.clone()],
        _ => Vec::new(),
    }
}

/// Bed size inferred from `printable_area` corner strings like `"300x270"`.
///
/// Returns `(width_mm, depth_mm)` as the max X and max Y seen in the polygon.
pub fn bed_size_mm(settings: &Value) -> Option<(f64, f64)> {
    let area = settings.get("printable_area")?.as_array()?;
    let mut max_x = 0.0_f64;
    let mut max_y = 0.0_f64;
    let mut any = false;
    for pt in area {
        let Some(s) = pt.as_str() else { continue };
        let Some((xs, ys)) = s.split_once('x') else {
            continue;
        };
        let Ok(x) = xs.parse::<f64>() else { continue };
        let Ok(y) = ys.parse::<f64>() else { continue };
        any = true;
        max_x = max_x.max(x);
        max_y = max_y.max(y);
    }
    if any { Some((max_x, max_y)) } else { None }
}

/// Colour graft policy (identity order):
///
/// After replacing `project_settings` with the template JSON, copy source
/// `filament_colour` and (if present on source) `filament_multi_colour` onto the
/// grafted value.
///
/// **Pad / truncate:** for each key, overwrite the first `min(source.len, dest.len)`
/// entries with source values. Do **not** invent colours beyond source length; do
/// **not** grow the template array. Excess source colours are ignored. Excess
/// destination slots keep template values.
///
/// Optionally copies `filament_type` labels with the same min-length policy.
/// Leaves template `filament_settings_id` and machine filament params untouched.
///
/// For non-identity slot maps, call [`reorder_filament_colours`] after this graft.
pub fn graft_filament_colours(
    template_settings: &mut Value,
    source_settings: &Value,
    copy_filament_type: bool,
) {
    patch_string_array_min_len(template_settings, source_settings, "filament_colour");
    if source_settings.get("filament_multi_colour").is_some() {
        // Only patch multi-colour when source has the key. If template lacks the key
        // but source has it, install a truncated copy sized to template filament count
        // or source length if template has no colour array.
        if template_settings.get("filament_multi_colour").is_none() {
            let src = string_array_field(source_settings, "filament_multi_colour");
            let dest_len = string_array_field(template_settings, "filament_colour").len();
            let take = if dest_len == 0 {
                src.len()
            } else {
                dest_len.min(src.len())
            };
            let arr: Vec<Value> = src.into_iter().take(take).map(Value::String).collect();
            if let Some(obj) = template_settings.as_object_mut() {
                obj.insert("filament_multi_colour".to_string(), Value::Array(arr));
            }
        } else {
            patch_string_array_min_len(template_settings, source_settings, "filament_multi_colour");
        }
    }
    if copy_filament_type {
        patch_string_array_min_len(template_settings, source_settings, "filament_type");
    }
}

/// Warnings produced while reordering colours under a non-identity map.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ColourReorderWarnings {
    pub messages: Vec<String>,
}

/// Reorder grafted `filament_colour` / `filament_multi_colour` so destination
/// toolhead `d` receives the source colour of preimage `s` where `map(s)=d`.
///
/// **C4:** bounds-safe pad (pad last colour or `#FFFFFF`), never panic.
/// Many-to-one: first source ascending wins + warning.
/// Does **not** shuffle `filament_settings_id`.
///
/// No-op when `map.is_identity()`.
pub fn reorder_filament_colours(
    grafted: &mut Value,
    source_settings: &Value,
    map: &SlotMap,
) -> ColourReorderWarnings {
    let mut warnings = ColourReorderWarnings::default();
    if map.is_identity() {
        return warnings;
    }

    // Source colours in identity order (0-based index = slot-1).
    let source_colours = string_array_field(source_settings, "filament_colour");
    let source_multi = string_array_field(source_settings, "filament_multi_colour");

    // Used sources: any source colour index present, or explicit map keys.
    let mut used: Vec<u8> = (1u8..=source_colours.len().min(16) as u8).collect();
    for (s, _) in map.iter() {
        if !used.contains(&s) {
            used.push(s);
        }
    }
    used.sort_unstable();
    used.dedup();

    // Many-to-one warnings
    for dest in map.many_to_one_dests(used.iter().copied()) {
        let preimages: Vec<u8> = used
            .iter()
            .copied()
            .filter(|&s| map.map_slot(s) == dest)
            .collect();
        warnings.messages.push(format!(
            "Many-to-one colour map into toolhead {dest}: sources {preimages:?}; \
             first ascending source wins"
        ));
    }

    // Build dest-slot (1-based) → colour from first preimage.
    // Only assign dests that have a preimage among used sources.
    let max_dest = used
        .iter()
        .map(|&s| map.map_slot(s))
        .filter(|&d| d >= 1)
        .max()
        .unwrap_or(0);

    if max_dest == 0 {
        return warnings;
    }

    reorder_colour_key(
        grafted,
        "filament_colour",
        &source_colours,
        map,
        &used,
        max_dest,
        &mut warnings,
    );
    if grafted.get("filament_multi_colour").is_some() || !source_multi.is_empty() {
        // Ensure multi key exists if we have source multi and grafted has colour.
        if grafted.get("filament_multi_colour").is_none()
            && !source_multi.is_empty()
            && let Some(obj) = grafted.as_object_mut()
        {
            let arr: Vec<Value> = source_multi.iter().cloned().map(Value::String).collect();
            obj.insert("filament_multi_colour".to_string(), Value::Array(arr));
        }
        if grafted.get("filament_multi_colour").is_some() {
            reorder_colour_key(
                grafted,
                "filament_multi_colour",
                &source_multi,
                map,
                &used,
                max_dest,
                &mut warnings,
            );
        }
    }

    warnings
}

fn reorder_colour_key(
    grafted: &mut Value,
    key: &str,
    source_colours: &[String],
    map: &SlotMap,
    used: &[u8],
    max_dest: u8,
    warnings: &mut ColourReorderWarnings,
) {
    // Work on a owned vec of current grafted colours (post identity graft).
    let mut colours = string_array_field(grafted, key);

    // Pad to max_dest (C4).
    let need = max_dest as usize;
    if colours.len() < need {
        let pad = colours
            .last()
            .cloned()
            .unwrap_or_else(|| "#FFFFFF".to_string());
        while colours.len() < need {
            colours.push(pad.clone());
        }
        warnings.messages.push(format!(
            "Padded {key} to length {need} for dest toolhead(s) (bounds-safe)"
        ));
    }

    // For each dest 1..=max_dest that has a preimage, assign source colour of first preimage.
    for dest in 1..=max_dest {
        let Some(src) = map.first_preimage(dest, used.iter().copied()) else {
            // Unmapped dest: keep grafted/template value.
            continue;
        };
        let src_idx = (src as usize).saturating_sub(1);
        let colour = source_colours
            .get(src_idx)
            .cloned()
            .or_else(|| colours.get(src_idx).cloned())
            .unwrap_or_else(|| "#FFFFFF".to_string());
        let dest_idx = (dest as usize).saturating_sub(1);
        // C4: ensure capacity then assign (already padded).
        if dest_idx >= colours.len() {
            let pad = colours
                .last()
                .cloned()
                .unwrap_or_else(|| "#FFFFFF".to_string());
            while colours.len() <= dest_idx {
                colours.push(pad.clone());
            }
        }
        if let Some(slot) = colours.get_mut(dest_idx) {
            *slot = colour;
        }
    }

    // Write back.
    if let Some(obj) = grafted.as_object_mut() {
        let arr: Vec<Value> = colours.into_iter().map(Value::String).collect();
        obj.insert(key.to_string(), Value::Array(arr));
    }
}

fn patch_string_array_min_len(dest: &mut Value, source: &Value, key: &str) {
    let src_vals = string_array_field(source, key);
    if src_vals.is_empty() {
        return;
    }
    let Some(Value::Array(dest_arr)) = dest.get_mut(key) else {
        // If dest lacks the key entirely, install source array as-is (capped by src).
        if let Some(obj) = dest.as_object_mut() {
            let arr: Vec<Value> = src_vals.into_iter().map(Value::String).collect();
            obj.insert(key.to_string(), Value::Array(arr));
        }
        return;
    };
    let n = src_vals.len().min(dest_arr.len());
    for i in 0..n {
        dest_arr[i] = json!(src_vals[i]);
    }
}

#[cfg(test)]
#[allow(non_snake_case)] // track test naming: feature__condition__expected
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn graft_filament_colours__source_differs__dest_gets_source_colours() {
        let mut template = json!({
            "printer_model": "WonderMaker ZR Ultra S",
            "filament_colour": ["#111111", "#222222", "#333333", "#444444"],
            "filament_settings_id": ["A", "B", "C", "D"],
            "filament_type": ["PETG", "PETG", "PETG", "PETG"],
        });
        let source = json!({
            "printer_model": "Bambu Lab H2C",
            "filament_colour": ["#FFFFFF", "#FFFF00", "#FF0000", "#0000FF"],
            "filament_multi_colour": ["#FFFFFF", "#FFFF00", "#FF0000", "#0000FF"],
            "filament_type": ["PLA", "PLA", "PLA", "PLA"],
        });
        graft_filament_colours(&mut template, &source, true);
        assert_eq!(
            string_array_field(&template, "filament_colour"),
            vec!["#FFFFFF", "#FFFF00", "#FF0000", "#0000FF"]
        );
        assert_eq!(
            string_array_field(&template, "filament_multi_colour"),
            vec!["#FFFFFF", "#FFFF00", "#FF0000", "#0000FF"]
        );
        assert_eq!(
            string_array_field(&template, "filament_type"),
            vec!["PLA", "PLA", "PLA", "PLA"]
        );
        // Machine filament ids stay template
        assert_eq!(
            string_array_field(&template, "filament_settings_id"),
            vec!["A", "B", "C", "D"]
        );
        assert_eq!(
            string_field(&template, "printer_model").as_deref(),
            Some("WonderMaker ZR Ultra S")
        );
    }

    #[test]
    fn graft_filament_colours__min_len_truncate__no_invented_colours() {
        let mut template = json!({
            "filament_colour": ["#AAAAAA", "#BBBBBB", "#CCCCCC", "#DDDDDD"],
        });
        let source = json!({
            "filament_colour": ["#111111", "#222222"],
        });
        graft_filament_colours(&mut template, &source, false);
        assert_eq!(
            string_array_field(&template, "filament_colour"),
            vec!["#111111", "#222222", "#CCCCCC", "#DDDDDD"]
        );
    }

    #[test]
    fn bed_size_mm__printable_area__max_corners() {
        let s = json!({
            "printable_area": ["0x0", "300x0", "300x270", "0x270"]
        });
        assert_eq!(bed_size_mm(&s), Some((300.0, 270.0)));
    }

    #[test]
    fn colour_reorder__swap_map() {
        let mut grafted = json!({
            "filament_colour": ["#FFFFFF", "#FFFF00", "#FF0000", "#0000FF"],
            "filament_settings_id": ["A", "B", "C", "D"],
        });
        // After identity graft, colours match source W,Y,R,B
        let source = json!({
            "filament_colour": ["#FFFFFF", "#FFFF00", "#FF0000", "#0000FF"],
        });
        let map = SlotMap::from_pairs([(1, 2), (2, 1), (3, 3), (4, 4)]);
        let _ = reorder_filament_colours(&mut grafted, &source, &map);
        assert_eq!(
            string_array_field(&grafted, "filament_colour"),
            vec!["#FFFF00", "#FFFFFF", "#FF0000", "#0000FF"],
            "swap 1↔2 should yield Y,W,R,B"
        );
        // filament_settings_id must not shuffle
        assert_eq!(
            string_array_field(&grafted, "filament_settings_id"),
            vec!["A", "B", "C", "D"]
        );
    }

    #[test]
    fn colour_reorder__short_array__pads_no_panic() {
        let mut grafted = json!({
            "filament_colour": ["#111111"],
        });
        let source = json!({
            "filament_colour": ["#111111"],
        });
        // Map source 1 → dest 4; array must pad, never panic.
        let map = SlotMap::from_pairs([(1, 4)]);
        let warnings = reorder_filament_colours(&mut grafted, &source, &map);
        let colours = string_array_field(&grafted, "filament_colour");
        assert!(
            colours.len() >= 4,
            "expected pad to dest 4, got {colours:?}"
        );
        assert_eq!(colours[3], "#111111");
        assert!(
            warnings
                .messages
                .iter()
                .any(|m| m.to_lowercase().contains("pad")),
            "expected pad warning, got {:?}",
            warnings.messages
        );
    }
}
