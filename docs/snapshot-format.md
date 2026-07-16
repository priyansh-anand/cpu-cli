# Snapshot format

A snapshot is a recording of the raw data `cpu` reads from a machine. `cpu --dump` writes one; `cpu --from` and the test suite read them. Because every collector reads through the same source traits, a snapshot replays exactly like the machine it came from.

A snapshot is either a **directory** (how fixtures are stored in `tests/fixtures/`) or a **`.tar.gz`** of the same files at the archive root (how `--dump` writes it).

## Layout

```
meta.toml       required: what was captured, and by which version of cpu
sysctl.toml     optional: sysctl keys and values (macOS)
ioreg.toml      optional: IOKit data properties as hex strings keyed service:key (macOS; only pmgr:voltage-states*)
fs/...          optional: file contents, at their absolute path under fs/ (Linux)
```

### `meta.toml`

```toml
snapshot_version = 1
cpu_version = "0.1.0"
os = "macos"            # std::env::consts::OS of the captured machine
arch = "aarch64"        # std::env::consts::ARCH
kernel = "25.5.0"       # optional
created_unix = 1790208000
note = "Apple M5"       # optional, free text; hand-written fixtures start with "SYNTHETIC:"
```

`os` selects the collector on replay, so a Linux snapshot replays as Linux on a Mac.

### `sysctl.toml`

A flat table of quoted keys. Integers are TOML integers; strings are strings; kernel arrays are stored as space-separated strings, as `sysctl` prints them:

```toml
"machdep.cpu.brand_string" = "Apple M5"
"hw.perflevel0.l2cachesize" = 16777216
"hw.cachesize" = "3629629440 65536 6291456 0 0 0 0 0 0 0"
```

### `fs/`

File contents keyed by absolute path: `fs/proc/cpuinfo` holds `/proc/cpuinfo`. Files are stored as UTF-8 text.

A `.tar.gz` whose files sit inside one top-level folder (for example after re-packing an extracted snapshot) opens too. An archive whose snapshot files come from more than one place (two folders, or the root and a folder) holds more than one snapshot and is refused, rather than merged into a CPU that doesn't exist.

## What gets captured

Capture uses a fixed **allowlist**, defined in `src/source/dump.rs`:

| Source | Captured |
|---|---|
| sysctl (macOS) | every key under `hw.*` and `machdep.cpu.*`, plus `sysctl.proc_translated` and `kern.osrelease` |
| IOKit (macOS) | the power manager's `voltage-states*` frequency tables (`pmgr`), nothing else from the registry |
| files (Linux) | `/proc/cpuinfo` (with `Serial` lines removed), `/proc/sys/kernel/{arch,osrelease}`, CPU and node lists, the hybrid core-type lists (`/sys/devices/cpu_{core,atom,lowpower}/cpus`), per-CPU topology, capacity, MIDR and cpufreq files, per-cache `level`, `type`, `size` and sharing files, and the DMI vendor and product name |

Linux files are listed one by one rather than swept by directory, because sysfs also contains kernel addresses (for example `crash_notes`).

The allowlist is a privacy guarantee: a snapshot never contains the hostname, serial numbers, hardware UUIDs or anything else outside it. It also deliberately captures more than today's collector reads (every `hw.*` and `machdep.cpu.*` sysctl, and every listed Linux file even if unused), so that when a future version reads more, old snapshots already contain it. On Linux the list is per file rather than per directory; see above.

## Versioning

- `snapshot_version` is checked before anything else is parsed. A snapshot from a newer format fails with `unsupported snapshot version N` instead of a confusing parse error.
- Adding new files or keys to the allowlist does **not** change the version; readers ignore what they don't use.
- Changing the meaning or layout of an existing file does.

## Using snapshots as fixtures

See [CONTRIBUTING.md](../CONTRIBUTING.md#add-a-machine-fixture). Unpack a dump into `tests/fixtures/<machine>/` and it is automatically covered by the golden, schema and invariant tests.
