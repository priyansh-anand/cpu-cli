//! Compiles `data/*.toml` into static Rust tables, rejecting bad data at build time so it can
//! never ship: duplicate keys, unknown groups or implementers, and missing fields.

use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::{env, fs};

use serde::Deserialize;

fn main() {
    let out_dir = PathBuf::from(env::var("OUT_DIR").expect("cargo sets OUT_DIR"));
    features(&out_dir);
    arm_midr(&out_dir);
}

#[derive(Deserialize)]
struct FeatureFile {
    feature: Vec<FeatureEntry>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct FeatureEntry {
    raw: String,
    name: String,
    family: Option<String>,
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

fn features(out_dir: &Path) {
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
        if let Some(family) = &e.family {
            assert!(
                !family.is_empty(),
                "data/features.toml: {:?} has an empty family",
                e.raw
            );
        }
        out.push_str(&format!(
            "    FeatureDef {{ raw: {:?}, name: {:?}, family: {:?}, group: FeatureGroup::{variant}, desc: {:?} }},\n",
            e.raw, e.name, e.family, e.desc
        ));
    }
    out.push_str("];\n");
    fs::write(out_dir.join("features.rs"), out).expect("write generated features.rs");
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct MidrFile {
    #[serde(rename = "ref")]
    reference: String,
    stamp: String,
    implementer: Vec<Implementer>,
    part: Vec<Part>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Implementer {
    id: u32,
    name: String,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Part {
    implementer: u32,
    id: u32,
    name: String,
}

fn arm_midr(out_dir: &Path) {
    println!("cargo::rerun-if-changed=data/arm-midr.toml");
    let text = fs::read_to_string("data/arm-midr.toml").expect("read data/arm-midr.toml");
    let file: MidrFile =
        toml::from_str(&text).unwrap_or_else(|e| panic!("data/arm-midr.toml: {e}"));
    assert!(
        !file.reference.is_empty() && !file.stamp.is_empty(),
        "data/arm-midr.toml needs a ref and a stamp"
    );

    let mut implementers = HashSet::new();
    let mut out = format!(
        "pub const ARM_MIDR_STAMP: &str = {:?};\n\npub static ARM_IMPLEMENTERS: &[(u32, &str)] = &[\n",
        file.stamp
    );
    for i in &file.implementer {
        assert!(
            implementers.insert(i.id),
            "data/arm-midr.toml: duplicate implementer {:#x}",
            i.id
        );
        out.push_str(&format!("    ({:#x}, {:?}),\n", i.id, i.name));
    }
    out.push_str("];\n\npub static ARM_PARTS: &[(u32, u32, &str)] = &[\n");
    let mut parts = HashSet::new();
    for p in &file.part {
        assert!(
            implementers.contains(&p.implementer),
            "data/arm-midr.toml: part {:#x} names unknown implementer {:#x}",
            p.id,
            p.implementer
        );
        assert!(
            parts.insert((p.implementer, p.id)),
            "data/arm-midr.toml: duplicate part {:#x}/{:#x}",
            p.implementer,
            p.id
        );
        out.push_str(&format!(
            "    ({:#x}, {:#x}, {:?}),\n",
            p.implementer, p.id, p.name
        ));
    }
    out.push_str("];\n");
    fs::write(out_dir.join("arm_midr.rs"), out).expect("write generated arm_midr.rs");
}
