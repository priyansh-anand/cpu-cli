//! `cpu`: a modern, pretty CPU viewer.
//!
//! The library keeps the binary organised and testable; it is not a stable public API.
//! The public contract is the `--json` output, described by `schema/cpu.v1.json`.

pub mod model;
pub mod units;

pub mod collect;
pub mod db;
pub mod render;
pub mod source;

#[cfg(test)]
pub(crate) mod test_support {
    use crate::collect::collect;
    use crate::model::Cpu;
    use crate::source::Sources;

    /// Loads `tests/fixtures/<name>` through the real snapshot + collector path.
    pub fn fixture(name: &str) -> Cpu {
        let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("tests/fixtures")
            .join(name);
        collect(&Sources::recorded(&path).unwrap_or_else(|e| panic!("{name}: {e}")))
    }
}
