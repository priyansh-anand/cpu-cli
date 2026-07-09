//! `--explain`: every value with where it came from, then anything that was rejected.

use serde_json::Value;

use super::view::{column_widths, pad};
use crate::model::Cpu;

pub fn render(cpu: &Cpu) -> String {
    let doc = serde_json::to_value(cpu).expect("the model is plain data");
    let mut rows = Vec::new();
    walk("", &doc, &mut rows);
    let mut out = String::from("Where each value came from\n\n");
    push_table(&mut out, &rows);
    if !cpu.diagnostics.is_empty() {
        let diagnostics: Vec<Vec<String>> = cpu
            .diagnostics
            .iter()
            .map(|d| vec![ascii(&d.from), ascii(&d.message)])
            .collect();
        out.push_str("\nDiagnostics\n\n");
        push_table(&mut out, &diagnostics);
    }
    out
}

/// Collects `path, value, origin [from]` for every fact under `v`.
fn walk(path: &str, v: &Value, rows: &mut Vec<Vec<String>>) {
    match v {
        Value::Object(map) if map.contains_key("value") && map.contains_key("origin") => {
            let value = match &map["value"] {
                Value::String(s) => s.clone(),
                other => other.to_string(),
            };
            let origin = map["origin"].as_str().unwrap_or_default();
            let origin = match map.get("from").and_then(Value::as_str) {
                Some(from) => format!("{origin}  {from}"),
                None => origin.to_string(),
            };
            rows.push(vec![ascii(path), ascii(&value), ascii(&origin)]);
        }
        Value::Object(map) => {
            for (key, child) in map {
                if key != "diagnostics" {
                    let child_path = if path.is_empty() {
                        key.clone()
                    } else {
                        format!("{path}.{key}")
                    };
                    walk(&child_path, child, rows);
                }
            }
        }
        Value::Array(items) => {
            for (i, child) in items.iter().enumerate() {
                walk(&format!("{path}[{i}]"), child, rows);
            }
        }
        _ => {}
    }
}

fn push_table(out: &mut String, rows: &[Vec<String>]) {
    let table: Vec<Vec<&str>> = rows
        .iter()
        .map(|r| r.iter().map(String::as_str).collect())
        .collect();
    let widths = column_widths(&table);
    for row in &table {
        let cells: Vec<String> = row.iter().zip(&widths).map(|(c, w)| pad(c, *w)).collect();
        out.push_str(format!("  {}", cells.join("  ")).trim_end());
        out.push('\n');
    }
}

fn ascii(s: &str) -> String {
    s.chars()
        .map(|c| {
            if c.is_ascii() && !c.is_control() {
                c
            } else {
                '?'
            }
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{Diagnostic, Fact};
    use crate::test_support::fixture;

    #[test]
    fn lists_every_value_with_its_origin() {
        let text = render(&fixture("apple-m5"));
        assert!(text.starts_with("Where each value came from\n\n"), "{text}");
        let line = text
            .lines()
            .find(|l| l.trim_start().starts_with("identity.name "))
            .expect("name row");
        assert!(
            line.contains("Apple M5") && line.contains("detected  sysctl:machdep.cpu.brand_string"),
            "{line}"
        );
        assert!(text.contains("clusters[0].clock.max"), "{text}");
        assert!(text.is_ascii());
        assert!(!text.contains("Diagnostics"));
    }

    #[test]
    fn diagnostics_follow_the_values() {
        let mut cpu = crate::model::Cpu::default();
        cpu.topology.logical_cpus = Some(Fact::derived(4));
        cpu.diagnostics.push(Diagnostic {
            from: "sysctl:hw.x".into(),
            message: "not a number".into(),
        });
        let text = render(&cpu);
        assert!(
            text.contains("\nDiagnostics\n\n  sysctl:hw.x  not a number\n"),
            "{text}"
        );
        assert!(text.contains("topology.logical_cpus  4  derived"), "{text}");
    }
}
