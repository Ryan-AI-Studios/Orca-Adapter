//! Filament/toolhead slot mapping.
//!
//! Source slots are 1-based filament indices (1..=16 for Bambu/Orca paint).
//! Destination slots target ZR Ultra-S toolheads (1..=4).

use std::collections::{BTreeMap, BTreeSet};

use crate::error::{Error, Result};

/// Source 1-based filament/toolhead index → destination 1-based toolhead.
///
/// Identity maps leave extruder attrs and paint codes unchanged.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SlotMap {
    /// Explicit mappings; missing keys imply identity for that source slot.
    map: BTreeMap<u8, u8>,
}

impl SlotMap {
    /// Empty map → all slots identity-mapped.
    pub fn identity() -> Self {
        Self {
            map: BTreeMap::new(),
        }
    }

    /// Build from explicit pairs (source → destination), both 1-based.
    ///
    /// Does **not** validate ranges — prefer [`SlotMap::parse`] or [`SlotMap::from_pairs_validated`]
    /// for user-facing input.
    pub fn from_pairs(pairs: impl IntoIterator<Item = (u8, u8)>) -> Self {
        Self {
            map: pairs.into_iter().collect(),
        }
    }

    /// Build from pairs with ZR-oriented validation (source 1..=16, dest 1..=4).
    pub fn from_pairs_validated(pairs: impl IntoIterator<Item = (u8, u8)>) -> Result<Self> {
        let mut map = BTreeMap::new();
        for (src, dest) in pairs {
            validate_source(src)?;
            validate_dest(dest)?;
            if map.insert(src, dest).is_some() {
                return Err(Error::msg(format!("duplicate source slot in map: {src}")));
            }
        }
        Ok(Self { map })
    }

    /// Parse a CLI-style map spec: `1=2,2=1,3=3,4=4` (whitespace optional).
    ///
    /// - Source slots must be 1..=16
    /// - Destination slots must be 1..=4 (ZR toolheads)
    /// - Duplicate source keys are rejected
    /// - Many-to-one destinations are allowed
    pub fn parse(spec: &str) -> Result<Self> {
        let trimmed = spec.trim();
        if trimmed.is_empty() {
            return Err(Error::msg(
                "slot map is empty; expected pairs like 1=2,2=1,3=3,4=4",
            ));
        }

        let mut map = BTreeMap::new();
        for token in trimmed.split(',') {
            let token = token.trim();
            if token.is_empty() {
                return Err(Error::msg(
                    "slot map has empty token (trailing/double comma?); expected pairs like 1=2",
                ));
            }
            let (src_s, dest_s) = token.split_once('=').ok_or_else(|| {
                Error::msg(format!(
                    "invalid slot map token '{token}'; expected SOURCE=DEST"
                ))
            })?;
            let src_s = src_s.trim();
            let dest_s = dest_s.trim();
            if src_s.is_empty() || dest_s.is_empty() {
                return Err(Error::msg(format!(
                    "invalid slot map token '{token}'; empty source or destination"
                )));
            }
            let src: u8 = src_s.parse().map_err(|_| {
                Error::msg(format!(
                    "invalid source slot '{src_s}' in token '{token}'; expected integer 1..=16"
                ))
            })?;
            let dest: u8 = dest_s.parse().map_err(|_| {
                Error::msg(format!(
                    "invalid destination slot '{dest_s}' in token '{token}'; expected integer 1..=4"
                ))
            })?;
            validate_source(src)?;
            validate_dest(dest)?;
            if map.insert(src, dest).is_some() {
                return Err(Error::msg(format!("duplicate source slot in map: {src}")));
            }
        }

        Ok(Self { map })
    }

    /// Look up destination for a 1-based source slot (defaults to identity).
    pub fn map_slot(&self, source_1based: u8) -> u8 {
        self.map
            .get(&source_1based)
            .copied()
            .unwrap_or(source_1based)
    }

    /// True when every defined mapping is identity (or map is empty).
    pub fn is_identity(&self) -> bool {
        self.map.iter().all(|(s, d)| s == d)
    }

    /// Iterate explicit mappings (sorted by source).
    pub fn iter(&self) -> impl Iterator<Item = (u8, u8)> + '_ {
        self.map.iter().map(|(&s, &d)| (s, d))
    }

    /// Explicit source keys in the map (sorted).
    pub fn used_sources(&self) -> Vec<u8> {
        self.map.keys().copied().collect()
    }

    /// All explicit pairs as a sorted vec.
    pub fn pairs(&self) -> Vec<(u8, u8)> {
        self.iter().collect()
    }

    /// Ensure every used source slot maps into ZR toolhead range 1..=4.
    ///
    /// Call with extruder/paint source slots actually present in the package.
    pub fn validate_used_map_to_zr(
        &self,
        used_sources: impl IntoIterator<Item = u8>,
    ) -> Result<()> {
        let mut bad: BTreeSet<(u8, u8)> = BTreeSet::new();
        for src in used_sources {
            if src == 0 {
                continue;
            }
            let dest = self.map_slot(src);
            if !(1..=4).contains(&dest) {
                bad.insert((src, dest));
            }
        }
        if bad.is_empty() {
            return Ok(());
        }
        let detail: Vec<String> = bad.iter().map(|(s, d)| format!("{s}→{d}")).collect();
        Err(Error::msg(format!(
            "used source slot(s) map outside ZR toolheads 1..=4: {}. \
             Provide an explicit map into 1..=4, or reduce colours (merge UX lands in a later track).",
            detail.join(", ")
        )))
    }

    /// Destinations that have more than one preimage among `used_sources` (many-to-one).
    pub fn many_to_one_dests(&self, used_sources: impl IntoIterator<Item = u8>) -> Vec<u8> {
        let mut dest_sources: BTreeMap<u8, Vec<u8>> = BTreeMap::new();
        for src in used_sources {
            if src == 0 {
                continue;
            }
            let dest = self.map_slot(src);
            dest_sources.entry(dest).or_default().push(src);
        }
        dest_sources
            .into_iter()
            .filter(|(_, srcs)| srcs.len() > 1)
            .map(|(d, _)| d)
            .collect()
    }

    /// First (ascending) source preimage that maps to `dest`, among `used_sources`.
    pub fn first_preimage(
        &self,
        dest: u8,
        used_sources: impl IntoIterator<Item = u8>,
    ) -> Option<u8> {
        let mut sources: Vec<u8> = used_sources
            .into_iter()
            .filter(|&s| s >= 1 && self.map_slot(s) == dest)
            .collect();
        sources.sort_unstable();
        sources.into_iter().next()
    }
}

impl Default for SlotMap {
    fn default() -> Self {
        Self::identity()
    }
}

fn validate_source(src: u8) -> Result<()> {
    if !(1..=16).contains(&src) {
        return Err(Error::msg(format!(
            "source slot {src} out of range (expected 1..=16)"
        )));
    }
    Ok(())
}

fn validate_dest(dest: u8) -> Result<()> {
    if !(1..=4).contains(&dest) {
        return Err(Error::msg(format!(
            "destination slot {dest} out of range (expected 1..=4 for ZR Ultra-S toolheads)"
        )));
    }
    Ok(())
}

#[cfg(test)]
#[allow(non_snake_case)] // track test naming: feature__condition__expected
mod tests {
    use super::*;

    #[test]
    fn slot_map__identity_default__maps_unchanged() {
        let m = SlotMap::identity();
        assert!(m.is_identity());
        assert_eq!(m.map_slot(1), 1);
        assert_eq!(m.map_slot(4), 4);
    }

    #[test]
    fn slot_map__explicit_swap__not_identity() {
        let m = SlotMap::from_pairs([(1, 2), (2, 1)]);
        assert!(!m.is_identity());
        assert_eq!(m.map_slot(1), 2);
        assert_eq!(m.map_slot(3), 3);
    }

    #[test]
    fn slot_map_parse__valid_and_invalid() {
        let m = SlotMap::parse("1=2,2=1,3=3,4=4").expect("valid");
        assert_eq!(m.map_slot(1), 2);
        assert_eq!(m.map_slot(2), 1);
        assert_eq!(m.map_slot(3), 3);
        assert_eq!(m.map_slot(4), 4);
        assert!(!m.is_identity());

        // Whitespace optional
        let m2 = SlotMap::parse(" 1 = 2 , 2 = 1 ").expect("ws");
        assert_eq!(m2.map_slot(1), 2);
        assert_eq!(m2.map_slot(2), 1);

        // Empty
        assert!(SlotMap::parse("").is_err());
        assert!(SlotMap::parse("   ").is_err());

        // Empty token
        assert!(SlotMap::parse("1=2,,3=3").is_err());

        // Non-u8 / non-integer
        assert!(SlotMap::parse("a=1").is_err());
        assert!(SlotMap::parse("1=b").is_err());

        // Source 0 or >16
        assert!(SlotMap::parse("0=1").is_err());
        assert!(SlotMap::parse("17=1").is_err());

        // Dest 0 or >4
        assert!(SlotMap::parse("1=0").is_err());
        assert!(SlotMap::parse("1=5").is_err());

        // Duplicate source
        assert!(SlotMap::parse("1=2,1=3").is_err());

        // Missing '='
        assert!(SlotMap::parse("1-2").is_err());

        // Many-to-one allowed
        let merge = SlotMap::parse("1=1,2=1,3=2,4=3").expect("many-to-one");
        assert_eq!(merge.map_slot(1), 1);
        assert_eq!(merge.map_slot(2), 1);
    }

    #[test]
    fn slot_map__validate_used_map_to_zr__rejects_out_of_range() {
        let m = SlotMap::identity();
        let err = m
            .validate_used_map_to_zr([1, 5])
            .expect_err("slot 5 identity is out of ZR range");
        assert!(err.to_string().contains("1..=4"));

        let ok = SlotMap::parse("1=1,2=2,5=4").expect("parse");
        ok.validate_used_map_to_zr([1, 2, 5])
            .expect("mapped into 4");
    }
}
