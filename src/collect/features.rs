//! Groups raw feature flags for display using `data/features.toml`.

use crate::db;
use crate::model::{Feature, FeatureGroupEntry, Features};

/// Keeps every raw flag; shows only flags the table knows, in table order.
pub fn group(raw: Vec<String>) -> Features {
    let mut groups: Vec<FeatureGroupEntry> = Vec::new();
    for def in db::FEATURES
        .iter()
        .filter(|def| raw.iter().any(|r| r == def.raw))
    {
        let feature = Feature {
            raw: def.raw.to_string(),
            name: def.name.to_string(),
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
        let f = group(vec![
            "FEAT_SHA3".into(),
            "AdvSIMD".into(),
            "FEAT_AES".into(),
            "FEAT_BTI".into(),
        ]);
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
        let f = group(vec!["FEAT_FUTURE".into(), "FEAT_AES".into()]);
        assert_eq!(f.raw, vec!["FEAT_FUTURE", "FEAT_AES"]);
        assert_eq!(f.groups.len(), 1);
    }

    #[test]
    fn no_flags_means_nothing_to_show() {
        assert!(group(Vec::new()).is_empty());
    }
}
