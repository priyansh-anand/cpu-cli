//! `--json`: the public, versioned contract. See `schema/cpu.v1.json`.

use serde::Serialize;

use crate::model::Cpu;

pub const SCHEMA_VERSION: u32 = 1;

#[derive(Serialize)]
struct Document<'a> {
    schema_version: u32,
    #[serde(flatten)]
    cpu: &'a Cpu,
}

pub fn render(cpu: &Cpu) -> String {
    let doc = Document {
        schema_version: SCHEMA_VERSION,
        cpu,
    };
    let mut text = serde_json::to_string_pretty(&doc).expect("the model is plain data");
    text.push('\n');
    text
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_support::fixture;

    #[test]
    fn apple_m5_json() {
        let text = render(&fixture("apple-m5"));
        assert!(text.starts_with("{\n  \"schema_version\": 1,"), "{text}");
        let v: serde_json::Value = serde_json::from_str(&text).unwrap();
        assert_eq!(v["clusters"][0]["name"]["value"], "Super");
        assert_eq!(v["clusters"][0]["caches"][2]["size"]["value"], 16_777_216);
        assert_eq!(
            v["identity"]["vendor"],
            serde_json::json!({"value": "Apple", "origin": "derived"})
        );
        assert!(v.get("shared_caches").is_none() && v.get("diagnostics").is_none());
        assert!(
            v["features"]["raw"]
                .as_array()
                .unwrap()
                .iter()
                .any(|f| f == "FEAT_FlagM")
        );
    }
}
