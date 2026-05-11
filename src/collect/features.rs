//! Groups raw feature flags for display using `data/features.toml`.

use crate::db::{self, Arch};
use crate::model::{Feature, FeatureGroupEntry, Features};

/// Keeps every raw flag; shows only flags the table knows for `arch`, in table order.
pub fn group(raw: Vec<String>, arch: Arch) -> Features {
    let mut groups: Vec<FeatureGroupEntry> = Vec::new();
    for def in db::FEATURES
        .iter()
        .filter(|def| def.arch.is_none_or(|a| a == arch))
        .filter(|def| raw.iter().any(|r| r == def.raw))
    {
        let feature = Feature {
            raw: def.raw.to_string(),
            name: def.name.to_string(),
            family: def.family.map(str::to_string),
        };
        match groups.iter_mut().find(|g| g.group == def.group) {
            Some(entry) => entry.features.push(feature),
            None => groups.push(FeatureGroupEntry {
                group: def.group,
                features: vec![feature],
            }),
        }
    }
    groups.sort_by_key(|g| g.group);
    Features { groups, raw }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::FeatureGroup;

    #[test]
    fn groups_known_flags_in_table_order() {
        let f = group(
            vec![
                "FEAT_SHA3".into(),
                "AdvSIMD".into(),
                "FEAT_AES".into(),
                "FEAT_BTI".into(),
            ],
            Arch::Arm,
        );
        let shown: Vec<(FeatureGroup, Vec<&str>)> = f
            .groups
            .iter()
            .map(|g| {
                (
                    g.group,
                    g.features.iter().map(|x| x.name.as_str()).collect(),
                )
            })
            .collect();
        assert_eq!(
            shown,
            vec![
                (FeatureGroup::Simd, vec!["NEON"]),
                (FeatureGroup::Crypto, vec!["AES", "SHA3"]),
                (FeatureGroup::Security, vec!["BTI"]),
            ]
        );
    }

    #[test]
    fn unknown_flags_are_kept_raw_but_not_shown() {
        let f = group(vec!["FEAT_FUTURE".into(), "FEAT_AES".into()], Arch::Arm);
        assert_eq!(f.raw, vec!["FEAT_FUTURE", "FEAT_AES"]);
        assert_eq!(f.groups.len(), 1);
    }

    #[test]
    fn no_flags_means_nothing_to_show() {
        assert!(group(Vec::new(), Arch::Arm).is_empty());
    }

    #[test]
    fn linux_flags_group_and_carry_their_family() {
        let f = group(
            vec![
                "avx512bw".into(),
                "avx2".into(),
                "avx512f".into(),
                "aes".into(),
                "vmx".into(),
            ],
            Arch::X86,
        );
        let simd = &f.groups[0];
        assert_eq!(simd.group, FeatureGroup::Simd);
        let shown: Vec<(&str, Option<&str>)> = simd
            .features
            .iter()
            .map(|x| (x.name.as_str(), x.family.as_deref()))
            .collect();
        assert_eq!(
            shown,
            [
                ("AVX2", None),
                ("F", Some("AVX-512")),
                ("BW", Some("AVX-512"))
            ]
        );
        let groups: Vec<FeatureGroup> = f.groups.iter().map(|g| g.group).collect();
        assert_eq!(
            groups,
            [
                FeatureGroup::Simd,
                FeatureGroup::Crypto,
                FeatureGroup::Virtualization
            ]
        );
    }

    #[test]
    fn flags_only_match_their_own_architecture() {
        let names = |f: &Features| -> Vec<String> {
            f.groups
                .iter()
                .flat_map(|g| g.features.iter().map(|x| x.name.clone()))
                .collect()
        };
        let x86 = group(vec!["sme".into(), "avx2".into(), "aes".into()], Arch::X86);
        assert_eq!(
            names(&x86),
            ["AVX2", "AES"],
            "AMD's sme is Secure Memory Encryption, not ARM SME"
        );
        assert_eq!(x86.raw, vec!["sme", "avx2", "aes"], "raw keeps every flag");
        let arm = group(vec!["sme".into(), "aes".into()], Arch::Arm);
        assert_eq!(names(&arm), ["SME", "AES"]);
    }
}
