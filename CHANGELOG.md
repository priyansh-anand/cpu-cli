# Changelog

All notable changes to this project are documented here. The format follows [Keep a Changelog](https://keepachangelog.com/en/1.1.0/), and the project uses [Semantic Versioning](https://semver.org). The `--json` output is versioned separately by `schema_version`; see [docs/json-output.md](docs/json-output.md).

## [Unreleased]

### Added

- macOS on Apple Silicon: CPU identity, per-core-type clusters (using the OS's own names, e.g. "Super"), L1/L2 caches with sharing and instance counts, and grouped feature flags.
- Boxed output for terminals, ASCII `--plain` output for pipes and non-UTF-8 locales, and `--color auto|always|never` honouring `NO_COLOR` and `CLICOLOR_FORCE`.
- `--json` output with `schema_version` 1 and a JSON Schema (`schema/cpu.v1.json`). Every value records its origin; unknown values are omitted.
- `--dump` to capture a privacy-safe snapshot of a machine's CPU data, and `--from` to replay one.
