#![allow(dead_code)]

use std::path::{Path, PathBuf};

use cpu_cli::collect::collect;
use cpu_cli::model::Cpu;
use cpu_cli::source::Sources;

pub fn fixture_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures")
}

/// Every fixture machine as `(name, path)`, sorted by name.
pub fn fixtures() -> Vec<(String, PathBuf)> {
    let mut all: Vec<(String, PathBuf)> = std::fs::read_dir(fixture_dir())
        .expect("tests/fixtures exists")
        .map(|entry| entry.expect("readable entry").path())
        .filter(|path| path.is_dir())
        .map(|path| {
            (
                path.file_name().unwrap().to_string_lossy().into_owned(),
                path,
            )
        })
        .collect();
    all.sort();
    assert!(!all.is_empty(), "no fixtures under tests/fixtures");
    all
}

pub fn load(path: &Path) -> Cpu {
    let sources = Sources::recorded(path).unwrap_or_else(|e| panic!("{}: {e}", path.display()));
    collect(&sources)
}

pub fn load_named(name: &str) -> Cpu {
    load(&fixture_dir().join(name))
}

/// `s` without ANSI SGR escape sequences (`ESC [ ... m`).
pub fn strip_ansi(s: &str) -> String {
    let mut out = String::new();
    let mut chars = s.chars();
    while let Some(c) = chars.next() {
        if c == '\x1b' {
            for c in chars.by_ref() {
                if c == 'm' {
                    break;
                }
            }
        } else {
            out.push(c);
        }
    }
    out
}
