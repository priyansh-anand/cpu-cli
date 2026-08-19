---
name: Wrong output or a new machine
about: Report output that is wrong or missing, or add your machine to the test suite
title: ""
labels: machine-snapshot
---

**Machine:** <!-- e.g. MacBook Pro M3 Max, Raspberry Pi 5, AWS c7g.large -->

**What is wrong or missing?** <!-- leave empty if you are only adding a machine -->

**`cpu --version` and OS:** <!-- e.g. cpu 2.0.0 on macOS 26.1 -->

### Snapshot

Run `cpu --dump` and drag the file it prints (`cpu-dump-<date>.tar.gz`) into this issue.

A snapshot contains only CPU data, read from a fixed allowlist: on macOS, the `hw.*` and `machdep.cpu.*` sysctl keys and the power manager's frequency tables; on Linux, `/proc/cpuinfo` without serial numbers, the kernel release and architecture, CPU topology, cache and clock files under `/sys`, and the DMI system vendor and product name. It never contains your hostname, serial numbers, hardware UUIDs or any of your files. List its contents with `tar -tzf cpu-dump-*.tar.gz`, or replay it with `cpu --from cpu-dump-*.tar.gz`.
