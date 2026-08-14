# Changelog

All notable changes to this project are documented here. The format follows [Keep a Changelog](https://keepachangelog.com/en/1.1.0/), and the project uses [Semantic Versioning](https://semver.org). The `--json` output is versioned separately by `schema_version`; see [docs/json-output.md](docs/json-output.md).

## [Unreleased]

### Added

- macOS on Apple Silicon: CPU identity, per-core-type clusters (using the OS's own names, e.g. "Super"), L1/L2 caches with sharing and instance counts, and grouped feature flags.
- Boxed output for terminals, ASCII `--plain` output for pipes and non-UTF-8 locales, and `--color auto|always|never` honouring `NO_COLOR` and `CLICOLOR_FORCE`.
- `--json` output with `schema_version` 1 and a JSON Schema (`schema/cpu.v1.json`). Every value records its origin; unknown values are omitted.
- `--dump` to capture a privacy-safe snapshot of a machine's CPU data, and `--from` to replay one.
- Linux on x86-64 and ARM64: identity, Intel hybrid and ARM big.LITTLE core types, caches (including caches shared across core types), base and max clock speeds, NUMA nodes and hypervisor detection.
- ARM core names from a table generated from util-linux's `lscpu-arm.c`.
- `--dump` on Linux, with a per-file allowlist that never captures kernel addresses or board serial numbers.
- Hypervisor names for common virtual platforms (KVM/QEMU, Hyper-V, VMware, Amazon EC2, Google Compute Engine, Apple Virtualization).
- Apple Silicon max clock speed per core type, from the power manager's frequency tables (IOKit).
- Rosetta 2 detection: an x86 build on Apple Silicon reports the real chip and says it is translated.
- Intel Mac support: identity, caches with sharing from `hw.cacheconfig`, base clock and feature flags.
- `--explain`, listing every value with where it came from, then any rejected values.
- Values from built-in tables are marked `†` with a footnote.
- Tables too wide for 80 columns are shown as one row per core type.
- Release builds for macOS (Apple Silicon and Intel) and statically linked Linux (x86-64 and ARM64), a shell installer and a Homebrew tap (`brew install priyansh-anand/tap/cpu-cli`).
- An issue template for submitting a machine snapshot; crash and unidentified-CPU messages link to it.

### Changed

- An unsupported operating system is named in the error, with a link to the issue tracker.

### Fixed

- `--dump` no longer overwrites an existing file or panics when stdout is closed, and names files by UTC time.
- Snapshot errors are one line, and a snapshot re-packed from its extracted folder opens.
- A crash prints a short message asking for `cpu --dump` instead of a raw backtrace.
