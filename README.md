# cpu

**See what silicon you're actually running.** A fast, readable CPU inspector for the terminal, in the spirit of `duf`, `bat` and `eza`, for the one classic tool that never got a modern rewrite: `lscpu`.

```
╭ Identity ────────────────────────────────────────────────────────╮
│ Name          Apple M5                                           │
│ Vendor        Apple                                              │
│ Architecture  arm64                                              │
╰──────────────────────────────────────────────────────────────────╯
╭ Topology ────────────────────────────────────────────────────────╮
│ Cores     10 physical · 10 logical · no SMT                      │
│ Clusters  4 × Super · 6 × Efficiency                             │
╰──────────────────────────────────────────────────────────────────╯
╭ Cache ─┬──────────────────┬──────────────────────────────────────╮
│ Level  │ Super            │ Efficiency                           │
├────────┼──────────────────┼──────────────────────────────────────┤
│ L1i    │ 192 KiB / core   │ 128 KiB / core                       │
│ L1d    │ 128 KiB / core   │ 64 KiB / core                        │
│ L2     │ 16 MiB / 4 cores │ 6 MiB / 6 cores                      │
╰────────┴──────────────────┴──────────────────────────────────────╯
╭ Features ────────────────────────────────────────────────────────╮
│ SIMD      NEON · FP16 · BF16 · I8MM · DotProd · FHM · SME · SME2 │
│           SME2.1                                                 │
│ Crypto    AES · PMULL · SHA1 · SHA256 · SHA512 · SHA3 · CRC32    │
│ Security  PAC · BTI · MTE · DIT · SB · CSV2 · CSV3               │
│ Other     LSE · LSE2                                             │
╰──────────────────────────────────────────────────────────────────╯
```

> **Status: early development.** macOS on Apple Silicon is supported today. Linux (x86 and ARM) and Intel Macs are in progress; see [Platform support](#platform-support).

## Why

- **`lscpu` is hard to read**, and it doesn't exist on macOS at all. On a Mac the same facts are scattered across dozens of `sysctl` keys.
- **Modern CPUs aren't uniform.** Performance and efficiency cores, shared caches and chiplets make a flat key/value dump actively misleading. `cpu` groups cores by type and shows which caches are shared by how many cores.
- **It never prints a guess.** Every value records where it came from. Anything the OS doesn't report is left out, not filled with `0` or a plausible-looking default.

## Install

`cpu` is not yet published to crates.io or Homebrew. Build it from source with Rust 1.85 or newer:

```sh
git clone <this repository> cpu
cd cpu
cargo install --path .
```

This installs a single `cpu` binary into `~/.cargo/bin`. (The crates.io package will be named `cpu-cli`, because `cpu` is taken; the binary is still `cpu`.)

## Usage

```sh
cpu                      # boxed tables in a terminal
cpu --plain              # aligned ASCII text: no boxes, no colour
cpu --json               # machine-readable output (see below)
cpu --dump               # save this machine's raw CPU data for a bug report
cpu --from snapshot.tar.gz   # show a saved snapshot instead of this machine
```

| Option | Effect |
|---|---|
| `--json` | Print JSON (`schema_version` 1). Overrides every display option. |
| `--plain` | Print plain text: ASCII only, no box drawing, no escape codes. |
| `--color auto\|always\|never` | When to use colour. Default `auto`. |
| `--from <SNAPSHOT>` | Read a snapshot directory or `.tar.gz` instead of the live machine. |
| `--dump [FILE]` | Write a snapshot to `FILE` (default `./cpu-dump-<unix-time>.tar.gz`) and print its path. |

### Output modes, pipes and colour

`cpu` picks the right output for where it's going:

- **In a terminal** it draws coloured boxes.
- **In a pipe or file** (`cpu | grep L2`, `cpu > cpu.txt`) it switches to `--plain`, so the output greps and diffs cleanly.
- **In a non-UTF-8 locale** (`LANG=C`) it uses plain text, because box-drawing characters wouldn't render.
- `--color always` (or `CLICOLOR_FORCE=1`) keeps the boxes and colour even in a pipe, e.g. for `cpu --color always | less -R`.
- `NO_COLOR` disables colour but keeps the boxes, per [no-color.org](https://no-color.org).
- Closing the pipe early (`cpu | head -1`) is not an error.

### JSON

`--json` is the stable, scriptable interface. Each value carries its origin, and unknown values are **omitted, never `null`**:

```json
{
  "schema_version": 1,
  "identity": {
    "vendor": { "value": "Apple", "origin": "derived" },
    "name":   { "value": "Apple M5", "origin": "detected", "from": "sysctl:machdep.cpu.brand_string" },
    "arch":   { "value": "arm64", "origin": "detected", "from": "sysctl:hw.optional.arm64" }
  },
  "topology": { "...": "..." },
  "clusters": [ "..." ],
  "features": { "groups": [ "..." ], "raw": [ "..." ] }
}
```

```sh
# Cores per core type
cpu --json | jq -c '.clusters[] | {name: .name.value, cores: .cores.value}'
# {"name":"Super","cores":4}
# {"name":"Efficiency","cores":6}

# Is SME available?
cpu --json | jq '.features.raw | index("FEAT_SME") != null'
```

The full contract is [`schema/cpu.v1.json`](schema/cpu.v1.json); compatibility rules are in [docs/json-output.md](docs/json-output.md).

### Reporting a machine that looks wrong

```sh
cpu --dump
# ./cpu-dump-1790208000.tar.gz
```

Attach that file to an issue. A snapshot contains **only CPU data**, captured from a fixed allowlist (`sysctl` keys under `hw.*` and `machdep.cpu.*`, plus the OS release). It never includes your hostname, serial numbers or hardware UUIDs. Anyone can replay it exactly with `cpu --from`, and it becomes a permanent regression test. The format is documented in [docs/snapshot-format.md](docs/snapshot-format.md).

### Exit codes

| Code | Meaning |
|---|---|
| `0` | Output printed (some fields may be absent because the OS doesn't report them). |
| `1` | The CPU could not be identified, the snapshot could not be read, or the OS is not supported yet. |
| `2` | Invalid command-line usage. |

## Where values come from

Every value has one of three origins, visible in `--json`:

| Origin | Meaning | Example |
|---|---|---|
| `detected` | Read from this machine; `from` names the exact key or file. | `sysctl:hw.perflevel0.l2cachesize` |
| `derived` | Computed from other detected values. | SMT = logical ÷ physical CPUs |
| `database` | Filled from a built-in table because the OS doesn't report it; `from` names the table and its date. Planned: no databases ship yet. | `apple-chips@2026-09` |

If none of these can produce a value, the row is hidden.

## Platform support

| Platform | Status |
|---|---|
| macOS, Apple Silicon | ✅ Supported: identity, per-core-type clusters, L1/L2 caches, feature flags |
| Linux, x86-64 and ARM64 | 🚧 In progress |
| macOS, Intel | 🚧 Planned |
| Clock speeds (all platforms) | 🚧 Planned |
| Windows, BSD | Not yet planned |

On a platform that isn't supported yet, `cpu` exits with status 1 and a message instead of printing incomplete data.

## How it works

```mermaid
flowchart LR
    live[("Live OS<br/>sysctl")] --> sources
    snap[("Snapshot<br/>--from")] --> sources
    sources["Sources<br/>raw reads"] --> collectors["Collectors<br/>parse + sanity checks"]
    collectors --> model["model::Cpu"]
    model --> boxed["boxed"]
    model --> plain["plain"]
    model --> json["json"]
```

Sources are the only code that touches the operating system, and every source has a *recorded* twin that reads a snapshot. So the whole program, including its test suite, runs identically on a live machine and on a snapshot captured from someone else's hardware. See [docs/architecture.md](docs/architecture.md).

## Development

```sh
cargo build
cargo test                                  # unit, CLI, schema, golden and invariant tests
cargo fmt --check
cargo clippy --all-targets -- -D warnings
```

See [CONTRIBUTING.md](CONTRIBUTING.md) for the test layout, how to add a machine fixture, and how to review output snapshots.

## Roadmap

1. **Linux + x86**: `/proc` and `/sys` collector, CPUID, ARM core names, NUMA nodes, clock speeds.
2. **Complete + release**: Intel Macs and Rosetta 2, Apple Silicon clock speeds, `--explain` (show where every value came from), Homebrew and crates.io releases.
3. **Later**: Windows, theming, fleet auditing (`--check`).

Out of scope: live monitoring (use `btop`), benchmarking and overclocking.

## License

[GPL-3.0-or-later](LICENSE).
