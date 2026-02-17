mod common;

use cpu_cli::render::json;
use serde_json::json;

fn validator() -> jsonschema::Validator {
    let text = std::fs::read_to_string(concat!(env!("CARGO_MANIFEST_DIR"), "/schema/cpu.v1.json"))
        .unwrap();
    jsonschema::validator_for(&serde_json::from_str(&text).unwrap()).unwrap()
}

#[test]
fn every_fixture_matches_the_schema() {
    let v = validator();
    for (name, path) in common::fixtures() {
        let doc: serde_json::Value =
            serde_json::from_str(&json::render(&common::load(&path))).unwrap();
        let errors: Vec<String> = v.iter_errors(&doc).map(|e| e.to_string()).collect();
        assert!(errors.is_empty(), "{name}: {errors:#?}");
    }
}

#[test]
fn the_schema_is_not_vacuous() {
    let v = validator();
    assert!(v.is_valid(&json!({"schema_version": 1, "identity": {}, "topology": {}})));
    assert!(!v.is_valid(&json!({"schema_version": 2, "identity": {}, "topology": {}})));
    assert!(!v.is_valid(&json!({
        "schema_version": 1,
        "identity": {"nmae": {"value": "x", "origin": "derived"}},
        "topology": {}
    })));
    assert!(!v.is_valid(&json!({
        "schema_version": 1,
        "identity": {"name": {"value": "x", "origin": "guessed"}},
        "topology": {}
    })));
}
