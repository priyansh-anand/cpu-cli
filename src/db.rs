//! Built-in tables compiled from `data/*.toml` by `build.rs`.

use crate::model::FeatureGroup;

pub struct FeatureDef {
    pub raw: &'static str,
    pub name: &'static str,
    /// Display family, e.g. `AVX-512` for `avx512f`.
    pub family: Option<&'static str>,
    pub group: FeatureGroup,
    pub desc: &'static str,
}

include!(concat!(env!("OUT_DIR"), "/features.rs"));

pub fn feature(raw: &str) -> Option<&'static FeatureDef> {
    FEATURES.iter().find(|f| f.raw == raw)
}

include!(concat!(env!("OUT_DIR"), "/arm_midr.rs"));

/// The vendor name for an ARM MIDR implementer code, e.g. `0x41` is `ARM`.
pub fn arm_implementer(id: u32) -> Option<&'static str> {
    ARM_IMPLEMENTERS
        .iter()
        .find(|(i, _)| *i == id)
        .map(|(_, name)| *name)
}

/// The core name for an implementer and part, e.g. `0x41`/`0xd0c` is `Neoverse-N1`.
pub fn arm_part(implementer: u32, part: u32) -> Option<&'static str> {
    ARM_PARTS
        .iter()
        .find(|(i, p, _)| *i == implementer && *p == part)
        .map(|(_, _, name)| *name)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn looks_up_features_by_raw_flag() {
        let pac = feature("FEAT_PAuth").unwrap();
        assert_eq!((pac.name, pac.group), ("PAC", FeatureGroup::Security));
        assert!(feature("FEAT_NOT_REAL").is_none());
    }

    #[test]
    fn names_arm_cores_by_midr() {
        assert_eq!(arm_implementer(0x41), Some("ARM"));
        assert_eq!(arm_implementer(0x61), Some("Apple"));
        assert_eq!(arm_part(0x41, 0xd0c), Some("Neoverse-N1"));
        assert_eq!(arm_part(0x41, 0xd05), Some("Cortex-A55"));
        assert_eq!(arm_part(0x61, 0x023), Some("Firestorm-M1"));
        assert_eq!(arm_part(0xc0, 0xac3), Some("Ampere-1"));
        assert_eq!(ARM_MIDR_STAMP, "arm-midr@2026-09");
    }

    #[test]
    fn apples_virtual_placeholder_part_has_no_name() {
        assert_eq!(arm_part(0x61, 0x000), None);
    }
}
