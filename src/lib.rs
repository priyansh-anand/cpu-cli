//! `cpu`: a modern, pretty CPU viewer.
//!
//! The library keeps the binary organised and testable; it is not a stable public API.
//! The public contract is the `--json` output, described by `schema/cpu.v1.json`.

pub mod model;
pub mod units;

pub mod source;
