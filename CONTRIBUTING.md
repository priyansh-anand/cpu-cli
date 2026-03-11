# Contributing to cpu

Thanks for helping. The most valuable contribution is often not code at all: a snapshot of a machine we don't have (see [Add a machine fixture](#add-a-machine-fixture)).

## Setup

Requires Rust 1.85 or newer ([rustup](https://rustup.rs)).

```sh
cargo build
cargo test
```

Before every commit:

```sh
cargo fmt
cargo clippy --all-targets -- -D warnings
cargo test
```

CI will enforce all three once it's set up; the tree should always be clean against them.

## Project layout

```
src/
  main.rs            CLI: flags, mode selection, exit codes, --dump
  lib.rs             module list (internal library, not a public API)
  model.rs           the shared data model: Cpu, Fact, Origin, Cluster, Cache, Features
  units.rs           Bytes and Hertz with human-friendly Display
  db.rs              tables generated from data/*.toml at build time
  source/            the only code that touches the OS
    mod.rs           Sysctl / Fs / Cpuid / IoReg traits, Sources, Os
    macos.rs         live sysctl reads (macOS only)
    snapshot.rs      snapshot format: open, write, parse
    dump.rs          --dump capture and its privacy allowlist
  collect/           raw source data → model
    macos.rs         macOS / Apple Silicon collector
    features.rs      raw feature flags → display groups
  render/            model → text (pure functions)
    view.rs          model → sections; shared by boxed and plain
    boxed.rs  plain.rs  json.rs
    mod.rs           mode and colour selection
data/features.toml   feature flag display names and groups
schema/cpu.v1.json   JSON output contract
tests/               integration tests and fixtures (below)
docs/                architecture, JSON contract, snapshot format
```

Read [docs/architecture.md](docs/architecture.md) before changing how data flows.

## Tests

| Layer | Where | What it guards |
|---|---|---|
| Unit | `#[cfg(test)]` in each module | parsing, decoding, layout, mode selection |
| Collector | `tests/collect_macos.rs` | fixtures produce the expected model |
| CLI | `tests/cli.rs` | flags, exit codes, pipes, `--dump`/`--from` round trip |
| Schema | `tests/schema.rs` | every fixture's JSON matches `schema/cpu.v1.json` |
| Golden | `tests/snapshots.rs` + `tests/snapshots/*.snap` | exact output of every fixture in every mode |
| Invariants | `tests/invariants.rs` | rules that must hold for *any* machine |

Fixtures live in `tests/fixtures/<machine>/` and are discovered automatically: a new fixture is covered by the golden, schema and invariant tests with no code changes.

Tests run on fixtures, not the machine you're on, so they pass on any OS. The only exceptions are the live `sysctl` tests in `src/source/macos.rs` and the dump replay test in `tests/cli.rs`, which run on macOS only.

### Reviewing output changes

Golden outputs use [insta](https://insta.rs). When you intentionally change output:

```sh
INSTA_UPDATE=always cargo test --test snapshots
git diff tests/snapshots/          # review every changed line
```

(or `cargo install cargo-insta` and use `cargo insta review`). Never hand-edit a `.snap` file; if one looks wrong, fix the code.

## Add a machine fixture

This is how `cpu` learns about hardware the maintainers don't own.

1. On the machine: `cpu --dump`. It prints the path of a `.tar.gz`.
2. Unpack it into a new fixture directory, named after the hardware:
   ```sh
   mkdir tests/fixtures/apple-m4-max
   tar -xzf cpu-dump-*.tar.gz -C tests/fixtures/apple-m4-max
   ```
3. Add a `note = "..."` line to its `meta.toml` saying what the machine is. Mark hand-written fixtures `SYNTHETIC:`.
4. Run `INSTA_UPDATE=always cargo test`, check the new `tests/snapshots/*@apple-m4-max-*.snap` files show what the hardware really is, and commit the fixture together with its snapshots.

If an invariant fails on a real snapshot, that's a bug in `cpu` (or a very unusual machine). Please open an issue with the snapshot attached rather than weakening the invariant.

## Add or rename a feature flag

Edit `data/features.toml`:

```toml
[[feature]]
raw = "FEAT_SME2"      # exactly as the OS reports it
name = "SME2"          # what the table shows
group = "simd"         # simd | crypto | virtualization | security | other
desc = "Scalable Matrix Extension version 2"
```

Entries are shown in file order within their group. Flags without an entry still appear in `--json` under `features.raw`. The build fails on duplicate flags, unknown groups or missing fields.

## Code conventions

- **Never print a guess.** Model fields that can be unknown are `F<T>` (`Option<Fact<T>>`). If a value can't be read, or fails a sanity check, it is `None` plus a `Diagnostic`, never a default.
- **Record where values come from.** Use `Fact::detected(value, "sysctl:<key>")` with the exact key or path, or `Fact::derived(value)` for computed values.
- **Only `source/` touches the OS.** Collectors read through the source traits so they run on snapshots; renderers are pure functions of the model.
- **No panics on bad input.** Collection and snapshot parsing return `None` or an error on anything unexpected. `expect` is reserved for invariants of our own data (e.g. serialising the model).
- **Clippy is the style guide**, including inline format args (`format!("{name}")`).
- Comments explain *why* (a hardware quirk, a platform rule), not what the next line does.

## Commits and pull requests

- [Conventional Commits](https://www.conventionalcommits.org): `feat:`, `fix:`, `test:`, `docs:`, `chore:`.
- One logical change per commit, and every commit builds and passes the tests.
- For behaviour changes, add the test first and make sure it fails without your change.
- Changes to `--json` output follow [docs/json-output.md](docs/json-output.md). A breaking change needs a new `schema_version`.
- Changes to the snapshot format follow [docs/snapshot-format.md](docs/snapshot-format.md).

## License

By contributing you agree that your contributions are licensed under [GPL-3.0-or-later](LICENSE).
