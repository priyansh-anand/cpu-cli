# JSON output

`cpu --json` is the stable, machine-readable interface. The Rust library is internal; this document and [`schema/cpu.v1.json`](../schema/cpu.v1.json) are the contract.

## Shape

```json
{
  "schema_version": 1,
  "identity":      { "name": FACT, "vendor": FACT, "arch": FACT, ... },
  "topology":      { "sockets": FACT, "physical_cores": FACT, "logical_cpus": FACT, "smt_per_core": FACT },
  "clusters":      [ CLUSTER, ... ],
  "shared_caches": [ CACHE, ... ],
  "features":      { "groups": [ ... ], "raw": [ "FEAT_AES", ... ] },
  "diagnostics":   [ { "from": "sysctl:hw.perflevel0.l2cachesize", "message": "implausible cache size Int(0)" } ]
}
```

### Facts

Every measured value is a *fact*:

```json
{ "value": 16777216, "origin": "detected", "from": "sysctl:hw.perflevel0.l2cachesize" }
```

| Field | Meaning |
|---|---|
| `value` | The value. Sizes are bytes and frequencies are hertz, always as integers. |
| `origin` | `detected` (read from the machine), `derived` (computed from other values) or `database` (from a built-in table). |
| `from` | For `detected`: the exact key or path. For `database`: the table and its date, e.g. `apple-chips@2026-09`. Absent for `derived`. |

### Unknown values are omitted

A field the OS doesn't report is **left out, never `null`**. Test for presence, not for null:

```sh
cpu --json | jq 'has("shared_caches")'
cpu --json | jq '.clusters[0].clock.max.value // "unknown"'
```

`identity` and `topology` are always present (possibly empty objects). `clusters`, `shared_caches`, `features` and `diagnostics` are present only when non-empty.

### Clusters and caches

A cluster is a **group of cores of one type**, e.g. `Super` and `Efficiency` on an Apple M5, or a single `uniform` cluster on a CPU with one core type.

```json
{
  "kind": "performance",
  "name": { "value": "Super", "origin": "detected", "from": "sysctl:hw.perflevel0.name" },
  "cores":   { "value": 4, "origin": "detected", "from": "sysctl:hw.perflevel0.physicalcpu" },
  "threads": { "value": 4, "origin": "detected", "from": "sysctl:hw.perflevel0.logicalcpu" },
  "caches": [
    { "level": 2, "kind": "unified",
      "size":      { "value": 16777216, ... },
      "shared_by": { "value": 4, ... },
      "instances": { "value": 1, "origin": "derived" } }
  ]
}
```

- `kind` is `performance`, `efficiency` or `uniform`; `name` is the OS's own label when it has one.
- A cache's `size` is **one instance**. `shared_by` is how many logical CPUs share one instance, and `instances` is how many exist in the cluster. Total capacity is `size × instances`.

### Features

- `features.raw` lists every enabled flag exactly as the OS reports it (`FEAT_SME2` on macOS; `avx2` on Linux, planned). Use this for scripting.
- `features.groups` is the curated, display-oriented grouping shown in the table. Its membership and names may change between releases; don't script against it.

### Diagnostics

Values the OS reported but `cpu` rejected (a 0-byte cache, a negative core count) appear in `diagnostics` with the key they came from. They are informational; the rejected value is simply absent elsewhere in the output.

## Compatibility policy

- `schema_version` is incremented for any **breaking** change: removing or renaming a field, changing a field's type or units, or changing the meaning of an existing value.
- **Additive** changes keep the version: new fields, new enum values in `origin`, `kind` or `group`, new entries in `features.raw`. Consumers should ignore fields they don't know.
- Which fields are *present* depends on the machine and on which collectors exist; a new release may report fields an older one omitted. That is not a breaking change.
- `schema/cpu.v1.json` is deliberately **strict** (`additionalProperties: false`): it describes exactly what *this release* emits, so the test suite catches any accidental change to the output. When a release adds a field, the schema is updated in the same change without bumping `schema_version`. Don't validate `cpu`'s output against a pinned copy of the schema in production; parse leniently and ignore unknown fields.
