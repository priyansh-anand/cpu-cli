//! Built-in tables compiled from `data/*.toml` by `build.rs`.

use crate::model::FeatureGroup;

pub struct FeatureDef {
    pub raw: &'static str,
    pub name: &'static str,
    pub group: FeatureGroup,
    pub desc: &'static str,
}

include!(concat!(env!("OUT_DIR"), "/features.rs"));

pub fn feature(raw: &str) -> Option<&'static FeatureDef> {
    FEATURES.iter().find(|f| f.raw == raw)
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
}
