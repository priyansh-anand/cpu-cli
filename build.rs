//! Compiles `data/*.toml` into static Rust tables, rejecting bad data at build time so it can
//! never ship: duplicate flags, unknown groups, and missing names or descriptions.

use std::collections::HashSet;
use std::{env, fs, path::Path};

use serde::Deserialize;

#[derive(Deserialize)]
struct FeatureFile {
    feature: Vec<FeatureEntry>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct FeatureEntry {
    raw: String,
    name: String,
    group: String,
    desc: String,
}

const GROUPS: &[(&str, &str)] = &[
    ("simd", "Simd"),
    ("crypto", "Crypto"),
    ("virtualization", "Virtualization"),
    ("security", "Security"),
    ("other", "Other"),
];

fn main() {
    println!("cargo::rerun-if-changed=data/features.toml");
    let text = fs::read_to_string("data/features.toml").expect("read data/features.toml");
    let file: FeatureFile =
        toml::from_str(&text).unwrap_or_else(|e| panic!("data/features.toml: {e}"));

    let mut seen = HashSet::new();
    let mut out = String::from("pub static FEATURES: &[FeatureDef] = &[\n");
    for e in &file.feature {
        assert!(
            seen.insert(e.raw.clone()),
            "data/features.toml: duplicate raw flag {:?}",
            e.raw
        );
        assert!(
            !e.name.is_empty() && !e.desc.is_empty(),
            "data/features.toml: {:?} needs a name and a desc",
            e.raw
        );
        let variant = GROUPS
            .iter()
            .find(|(key, _)| *key == e.group)
            .map(|(_, variant)| *variant)
            .unwrap_or_else(|| {
                panic!(
                    "data/features.toml: {:?} has unknown group {:?}",
                    e.raw, e.group
                )
            });
        out.push_str(&format!(
            "    FeatureDef {{ raw: {:?}, name: {:?}, group: FeatureGroup::{variant}, desc: {:?} }},\n",
            e.raw, e.name, e.desc
        ));
    }
    out.push_str("];\n");

    let dest = Path::new(&env::var("OUT_DIR").expect("cargo sets OUT_DIR")).join("features.rs");
    fs::write(dest, out).expect("write generated features.rs");
}
