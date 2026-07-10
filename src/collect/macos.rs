//! macOS collector. Apple Silicon reports each core type as a "perflevel" (`hw.perflevel0` is the
//! fastest); each becomes one [`Cluster`]. Compiled on every OS so fixtures test it anywhere.

use crate::db::Arch;
use crate::model::{
    Cache, CacheKind, Clocks, Cluster, CoreKind, Cpu, Diagnostic, F, Fact, Identity, Topology,
};
use crate::source::{IoReg, Sysctl};
use crate::units::{Bytes, Hertz};

use super::{features, plausible_cache_size, ratio};

/// More perflevels than this is garbage, not a real chip.
const MAX_PERFLEVELS: u32 = 8;

pub fn collect(sys: &dyn Sysctl, ioreg: &dyn IoReg) -> Cpu {
    let mut r = Reader {
        sys,
        ioreg,
        diagnostics: Vec::new(),
    };
    // Apple Silicon, natively or under Rosetta, reports hw.optional.arm64 = 1.
    if !r.flag("hw.optional.arm64") && sys.string("machdep.cpu.vendor").is_some() {
        return intel(&mut r);
    }
    let identity = identity(&mut r);
    let topology = topology(&mut r);
    let clusters = clusters(&mut r, &topology);
    let features = features::group(arm_feature_flags(sys), Arch::Arm);
    Cpu {
        identity,
        topology,
        clusters,
        shared_caches: Vec::new(),
        features,
        diagnostics: r.diagnostics,
    }
}

/// Reads typed values, recording a [`Diagnostic`] for values that exist but make no sense.
/// A missing key is normal (older macOS, other chips) and is not a diagnostic.
struct Reader<'a> {
    sys: &'a dyn Sysctl,
    ioreg: &'a dyn IoReg,
    diagnostics: Vec<Diagnostic>,
}

impl Reader<'_> {
    fn text(&mut self, key: &str) -> F<String> {
        let value = self.sys.string(key)?;
        let value = value.trim();
        if value.is_empty() {
            self.reject(key, "empty value".to_string());
            return None;
        }
        // Snapshots can come from anyone; a control character could drive the reader's terminal.
        if value.chars().any(char::is_control) {
            self.reject(key, "contains control characters".to_string());
            return None;
        }
        Some(Fact::detected(value.to_string(), source(key)))
    }

    fn count(&mut self, key: &str) -> F<u32> {
        let raw = self.sys.get(key)?;
        match raw
            .as_i64()
            .and_then(|n| u32::try_from(n).ok())
            .filter(|n| *n > 0)
        {
            Some(n) => Some(Fact::detected(n, source(key))),
            None => {
                self.reject(key, format!("expected a positive count, got {raw:?}"));
                None
            }
        }
    }

    fn cache_size(&mut self, key: &str) -> F<Bytes> {
        let raw = self.sys.get(key)?;
        match raw
            .as_i64()
            .and_then(|n| u64::try_from(n).ok())
            .filter(|n| plausible_cache_size(*n))
        {
            Some(n) => Some(Fact::detected(Bytes(n), source(key))),
            None => {
                self.reject(key, format!("implausible cache size {raw:?}"));
                None
            }
        }
    }

    /// A perflevel's max clock from its `pmgr` table; only for two-perflevel chips.
    fn max_clock(&mut self, index: u32, levels: u32) -> F<Hertz> {
        let table = CLOCK_TABLES.get(index as usize).filter(|_| levels == 2)?;
        let bytes = self.ioreg.property("pmgr", table)?;
        let from = format!("ioreg:pmgr:{table}");
        match max_frequency(&bytes) {
            Some(hz) => Some(Fact::detected(Hertz(hz), from)),
            None => {
                self.diagnostics.push(Diagnostic {
                    from,
                    message: "implausible frequency table".to_string(),
                });
                None
            }
        }
    }

    /// A non-negative integer (a stepping may be 0).
    fn number(&mut self, key: &str) -> F<u32> {
        let raw = self.sys.get(key)?;
        match raw.as_i64().and_then(|n| u32::try_from(n).ok()) {
            Some(n) => Some(Fact::detected(n, source(key))),
            None => {
                self.reject(key, "expected a non-negative number".to_string());
                None
            }
        }
    }

    fn hertz(&mut self, key: &str) -> F<Hertz> {
        let raw = self.sys.get(key)?;
        match raw
            .as_i64()
            .and_then(|n| u64::try_from(n).ok())
            .filter(|n| (100_000_000..=10_000_000_000).contains(n))
        {
            Some(hz) => Some(Fact::detected(Hertz(hz), source(key))),
            None => {
                self.reject(key, "implausible frequency".to_string());
                None
            }
        }
    }

    fn flag(&self, key: &str) -> bool {
        self.sys.int(key) == Some(1)
    }

    fn reject(&mut self, key: &str, message: String) {
        self.diagnostics.push(Diagnostic {
            from: source(key),
            message,
        });
    }
}

/// Which `pmgr` table belongs to which perflevel: perflevel0 (fastest) is `voltage-states5-sram`,
/// perflevel1 is `voltage-states1-sram`, on every Apple Silicon generation so far.
const CLOCK_TABLES: [&str; 2] = ["voltage-states5-sram", "voltage-states1-sram"];

/// Highest frequency in a `voltage-states*` table of 8-byte (frequency, voltage) little-endian u32
/// entries. M1-M3 store hertz; the M5 stores kilohertz (4.46 GHz does not fit a u32 in hertz), so a
/// maximum below 100 000 000 means kilohertz. Entries are not sorted, so the maximum is taken.
/// `0xffffffff` entries are placeholders and are skipped; a length that is not a whole number of
/// entries means the table is damaged, so nothing is read from it.
pub(crate) fn max_frequency(table: &[u8]) -> Option<u64> {
    if table.len() % 8 != 0 {
        return None;
    }
    let max = table
        .chunks_exact(8)
        .map(|e| u32::from_le_bytes([e[0], e[1], e[2], e[3]]))
        .filter(|&f| f != u32::MAX)
        .map(u64::from)
        .max()?;
    let hz = if max >= 100_000_000 { max } else { max * 1000 };
    (100_000_000..=10_000_000_000).contains(&hz).then_some(hz)
}

/// Intel Macs: `machdep.cpu.*` identity and features, `hw.*` caches and clocks.
/// `hw.cacheconfig` lists the logical CPUs sharing each level (index 1 is L1, 2 is L2, 3 is L3),
/// so CPUID isn't needed.
fn intel(r: &mut Reader) -> Cpu {
    let identity = Identity {
        vendor: r.text("machdep.cpu.vendor").map(|f| {
            let value = if f.value == "GenuineIntel" {
                "Intel".to_string()
            } else {
                f.value
            };
            Fact {
                value,
                origin: f.origin,
            }
        }),
        name: r.text("machdep.cpu.brand_string"),
        arch: r
            .flag("hw.optional.x86_64")
            .then(|| Fact::detected("x86_64".to_string(), source("hw.optional.x86_64"))),
        x86_family: r.number("machdep.cpu.family"),
        x86_model: r.number("machdep.cpu.model"),
        x86_stepping: r.number("machdep.cpu.stepping"),
        microcode: r.number("machdep.cpu.microcode_version").map(|f| Fact {
            value: format!("{:#x}", f.value),
            origin: f.origin,
        }),
        ..Identity::default()
    };
    let topology = topology(r);
    let smt = topology.smt_per_core.as_ref().map_or(1, |f| f.value);
    let sharers: Vec<u32> = r
        .sys
        .string("hw.cacheconfig")
        .map(|s| {
            s.split_whitespace()
                .filter_map(|n| n.parse().ok())
                .collect()
        })
        .unwrap_or_default();
    let mut caches = Vec::new();
    for (key, level, kind, index) in [
        ("hw.l1icachesize", 1, CacheKind::Instruction, 1),
        ("hw.l1dcachesize", 1, CacheKind::Data, 1),
        ("hw.l2cachesize", 2, CacheKind::Unified, 2),
        ("hw.l3cachesize", 3, CacheKind::Unified, 3),
    ] {
        let Some(size) = r.cache_size(key) else {
            continue;
        };
        let shared_by = sharers
            .get(index)
            .copied()
            .filter(|n| *n > 0)
            .map(|n| Fact::detected(n, source("hw.cacheconfig")));
        let cores = shared_by
            .as_ref()
            .filter(|s| s.value % smt == 0)
            .map(|s| Fact::derived(s.value / smt));
        let instances = ratio(&topology.logical_cpus, &shared_by);
        caches.push(Cache {
            level,
            kind,
            size: Some(size),
            shared_by,
            cores,
            instances,
        });
    }
    let base = r.hertz("hw.cpufrequency");
    // Intel Macs usually report the nominal clock here, not turbo; only a higher value says anything.
    let max = r
        .hertz("hw.cpufrequency_max")
        .filter(|m| base.as_ref().is_none_or(|b| m.value > b.value));
    let cluster = Cluster {
        kind: CoreKind::Uniform,
        name: None,
        cores: topology.physical_cores.clone(),
        threads: topology.logical_cpus.clone(),
        clock: Clocks {
            base,
            max,
            current: None,
        },
        caches,
    };
    let flags: Vec<String> = [
        "machdep.cpu.features",
        "machdep.cpu.leaf7_features",
        "machdep.cpu.extfeatures",
    ]
    .iter()
    .filter_map(|key| r.sys.string(key))
    .flat_map(|s| s.split_whitespace().map(str::to_string).collect::<Vec<_>>())
    .collect();
    let mut flags = flags;
    flags.sort();
    flags.dedup();
    Cpu {
        identity,
        topology,
        clusters: vec![cluster],
        shared_caches: Vec::new(),
        features: features::group(flags, Arch::X86),
        diagnostics: std::mem::take(&mut r.diagnostics),
    }
}

fn source(key: &str) -> String {
    format!("sysctl:{key}")
}

fn identity(r: &mut Reader) -> Identity {
    let name = r.text("machdep.cpu.brand_string");
    let vendor = r.text("machdep.cpu.vendor").or_else(|| {
        name.as_ref()
            .filter(|n| n.value.starts_with("Apple "))
            .map(|_| Fact::derived("Apple".to_string()))
    });
    let arch = r
        .flag("hw.optional.arm64")
        .then(|| Fact::detected("arm64".to_string(), source("hw.optional.arm64")));
    // An x86 build under Rosetta still sees the real ARM keys (plus fake x86 ones this path never
    // reads); proc_translated says so.
    let translated = r
        .flag("sysctl.proc_translated")
        .then(|| Fact::detected(true, source("sysctl.proc_translated")));
    Identity {
        name,
        vendor,
        arch,
        translated,
        ..Identity::default()
    }
}

fn topology(r: &mut Reader) -> Topology {
    let physical_cores = r.count("hw.physicalcpu");
    let logical_cpus = r.count("hw.logicalcpu");
    let smt_per_core = ratio(&logical_cpus, &physical_cores);
    Topology {
        sockets: r.count("hw.packages"),
        physical_cores,
        logical_cpus,
        smt_per_core,
        numa_nodes: Vec::new(),
    }
}

fn clusters(r: &mut Reader, topology: &Topology) -> Vec<Cluster> {
    let levels = match r.count("hw.nperflevels") {
        Some(n) if n.value > MAX_PERFLEVELS => {
            r.reject(
                "hw.nperflevels",
                format!("{} perflevels is not plausible", n.value),
            );
            0
        }
        Some(n) => n.value,
        None => 0,
    };
    if levels > 0 {
        let mut clusters: Vec<Cluster> = (0..levels).map(|i| perflevel(r, i, levels)).collect();
        check_clocks(r, &mut clusters);
        return clusters;
    }
    // No perflevel keys (macOS 11): one cluster with counts only. The top-level hw.l*cachesize
    // keys are deliberately ignored: on Apple Silicon they describe only the efficiency cores.
    if topology.physical_cores.is_none() && topology.logical_cpus.is_none() {
        return Vec::new();
    }
    vec![Cluster {
        kind: CoreKind::Uniform,
        name: None,
        cores: topology.physical_cores.clone(),
        threads: topology.logical_cpus.clone(),
        clock: Clocks::default(),
        caches: Vec::new(),
    }]
}

/// The table-to-perflevel mapping is only known for a Performance + Efficiency pair, and it must
/// put the faster table on the performance cores. Anything else drops both clocks: a missing
/// clock is hidden, a wrong one is printed.
fn check_clocks(r: &mut Reader, clusters: &mut [Cluster]) {
    let [p, e] = clusters else { return };
    let (Some(p_max), Some(e_max)) = (&p.clock.max, &e.clock.max) else {
        return;
    };
    let problem = if e.name.as_ref().map(|n| n.value.as_str()) != Some("Efficiency") {
        "clock tables are only mapped for Performance + Efficiency chips"
    } else if p_max.value.0 <= e_max.value.0 {
        "performance clock table is slower than the efficiency one"
    } else {
        return;
    };
    r.diagnostics.push(Diagnostic {
        from: format!("ioreg:pmgr:{}", CLOCK_TABLES[0]),
        message: problem.to_string(),
    });
    p.clock.max = None;
    e.clock.max = None;
}

fn perflevel(r: &mut Reader, index: u32, levels: u32) -> Cluster {
    let key = |leaf: &str| format!("hw.perflevel{index}.{leaf}");
    let kind = match (levels, index) {
        (1, _) => CoreKind::Uniform,
        (_, i) if i + 1 == levels => CoreKind::Efficiency,
        _ => CoreKind::Performance,
    };
    let name = r.text(&key("name"));
    let cores = r.count(&key("physicalcpu"));
    let threads = r.count(&key("logicalcpu"));
    let per_l2 = r.count(&key("cpusperl2"));
    let per_core = ratio(&threads, &cores);
    // Without SMT every CPU is a core, so the CPUs sharing a cache are also its cores.
    let no_smt = per_core.as_ref().is_some_and(|f| f.value == 1);
    let mut caches = Vec::new();
    for (leaf, level, cache_kind, shared_by) in [
        ("l1icachesize", 1, CacheKind::Instruction, &per_core),
        ("l1dcachesize", 1, CacheKind::Data, &per_core),
        ("l2cachesize", 2, CacheKind::Unified, &per_l2),
    ] {
        let Some(size) = r.cache_size(&key(leaf)) else {
            continue;
        };
        caches.push(Cache {
            level,
            kind: cache_kind,
            size: Some(size),
            shared_by: shared_by.clone(),
            cores: if no_smt {
                shared_by.as_ref().map(|f| Fact::derived(f.value))
            } else {
                None
            },
            instances: ratio(&threads, shared_by),
        });
    }
    Cluster {
        kind,
        name,
        cores,
        threads,
        clock: Clocks {
            max: r.max_clock(index, levels),
            ..Clocks::default()
        },
        caches,
    }
}

/// Enabled `hw.optional.arm.*` flags, sorted so live and recorded runs match exactly.
fn arm_feature_flags(sys: &dyn Sysctl) -> Vec<String> {
    const PREFIX: &str = "hw.optional.arm.";
    let mut flags: Vec<String> = sys
        .keys_with_prefix(PREFIX)
        .into_iter()
        .filter(|key| sys.int(key) == Some(1))
        .filter_map(|key| key.strip_prefix(PREFIX).map(str::to_string))
        .collect();
    flags.sort();
    flags
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;

    use super::*;
    use crate::source::Stub;

    /// The collector with no IOKit tables, as the older tests expect.
    fn collect_sys(sys: &dyn Sysctl) -> Cpu {
        collect(sys, &Stub)
    }
    use crate::model::Origin;
    use crate::source::SysctlValue::{self, Int, Str};

    fn two_level_mac() -> Vec<(&'static str, SysctlValue)> {
        vec![
            ("machdep.cpu.brand_string", Str("Apple M5".into())),
            ("hw.optional.arm64", Int(1)),
            ("hw.physicalcpu", Int(10)),
            ("hw.logicalcpu", Int(10)),
            ("hw.packages", Int(1)),
            ("hw.nperflevels", Int(2)),
            ("hw.perflevel0.name", Str("Super".into())),
            ("hw.perflevel0.physicalcpu", Int(4)),
            ("hw.perflevel0.logicalcpu", Int(4)),
            ("hw.perflevel0.l1icachesize", Int(196_608)),
            ("hw.perflevel0.l1dcachesize", Int(131_072)),
            ("hw.perflevel0.l2cachesize", Int(16_777_216)),
            ("hw.perflevel0.cpusperl2", Int(4)),
            ("hw.perflevel1.name", Str("Efficiency".into())),
            ("hw.perflevel1.physicalcpu", Int(6)),
            ("hw.perflevel1.logicalcpu", Int(6)),
            ("hw.perflevel1.l1icachesize", Int(131_072)),
            ("hw.perflevel1.l1dcachesize", Int(65_536)),
            ("hw.perflevel1.l2cachesize", Int(6_291_456)),
            ("hw.perflevel1.cpusperl2", Int(6)),
            ("hw.l2cachesize", Int(6_291_456)),
            ("hw.optional.arm.FEAT_AES", Int(1)),
            ("hw.optional.arm.FEAT_SSBS", Int(0)),
        ]
    }

    /// `two_level_mac()` with some keys replaced (`Some`) or removed (`None`).
    fn with(changes: &[(&'static str, Option<SysctlValue>)]) -> BTreeMap<String, SysctlValue> {
        let mut pairs = two_level_mac();
        for (key, value) in changes {
            pairs.retain(|(k, _)| k != key);
            if let Some(value) = value {
                pairs.push((*key, value.clone()));
            }
        }
        pairs.into_iter().map(|(k, v)| (k.to_string(), v)).collect()
    }

    fn value<T: Clone>(fact: &F<T>) -> Option<T> {
        fact.as_ref().map(|f| f.value.clone())
    }

    #[test]
    fn perflevels_become_named_clusters_fastest_first() {
        let cpu = collect_sys(&with(&[]));
        let c = &cpu.clusters;
        assert_eq!(c.len(), 2);
        assert_eq!(
            (c[0].kind, c[0].label(), value(&c[0].cores)),
            (CoreKind::Performance, "Super", Some(4))
        );
        assert_eq!(
            (c[1].kind, c[1].label(), value(&c[1].cores)),
            (CoreKind::Efficiency, "Efficiency", Some(6))
        );
    }

    #[test]
    fn caches_record_size_sharing_and_instances() {
        let cpu = collect_sys(&with(&[]));
        let l2 = cpu.clusters[0]
            .caches
            .iter()
            .find(|k| k.level == 2)
            .unwrap();
        assert_eq!(value(&l2.size), Some(Bytes(16 << 20)));
        assert_eq!(
            l2.size.as_ref().unwrap().origin,
            Origin::Detected("sysctl:hw.perflevel0.l2cachesize".into())
        );
        assert_eq!(
            (value(&l2.shared_by), value(&l2.instances)),
            (Some(4), Some(1))
        );
        let l1d = cpu.clusters[1]
            .caches
            .iter()
            .find(|k| k.kind == CacheKind::Data)
            .unwrap();
        assert_eq!(
            (
                value(&l1d.size),
                value(&l1d.shared_by),
                value(&l1d.instances)
            ),
            (Some(Bytes(64 << 10)), Some(1), Some(6))
        );
    }

    #[test]
    fn two_physical_clusters_of_one_type_show_as_instances() {
        let cpu = collect_sys(&with(&[
            ("hw.perflevel0.physicalcpu", Some(Int(8))),
            ("hw.perflevel0.logicalcpu", Some(Int(8))),
        ]));
        let l2 = cpu.clusters[0]
            .caches
            .iter()
            .find(|k| k.level == 2)
            .unwrap();
        assert_eq!(
            (value(&l2.shared_by), value(&l2.instances)),
            (Some(4), Some(2))
        );
    }

    #[test]
    fn identity_and_topology() {
        let cpu = collect_sys(&with(&[]));
        assert_eq!(value(&cpu.identity.name).as_deref(), Some("Apple M5"));
        assert_eq!(
            cpu.identity.vendor,
            Some(Fact::derived("Apple".to_string()))
        );
        assert_eq!(value(&cpu.identity.arch).as_deref(), Some("arm64"));
        assert_eq!(value(&cpu.topology.smt_per_core), Some(1));
        assert_eq!(value(&cpu.topology.sockets), Some(1));
    }

    #[test]
    fn only_enabled_features_are_collected() {
        assert_eq!(collect_sys(&with(&[])).features.raw, vec!["FEAT_AES"]);
    }

    #[test]
    fn without_perflevels_there_is_one_cluster_and_no_guessed_caches() {
        let cpu = collect_sys(&with(&[("hw.nperflevels", None)]));
        assert_eq!(cpu.clusters.len(), 1);
        assert_eq!(cpu.clusters[0].kind, CoreKind::Uniform);
        assert!(
            cpu.clusters[0].caches.is_empty(),
            "top-level hw.l2cachesize describes E-cores only"
        );
    }

    #[test]
    fn implausible_values_are_dropped_with_a_diagnostic() {
        let cpu = collect_sys(&with(&[
            ("hw.perflevel0.physicalcpu", Some(Int(0))),
            ("hw.perflevel0.l2cachesize", Some(Int(0))),
            ("hw.perflevel1.l1dcachesize", Some(Int(1000))),
        ]));
        assert_eq!(cpu.clusters[0].cores, None);
        assert!(cpu.clusters[0].caches.iter().all(|k| k.level != 2));
        assert!(
            cpu.clusters[1]
                .caches
                .iter()
                .all(|k| k.kind != CacheKind::Data)
        );
        let from: Vec<&str> = cpu.diagnostics.iter().map(|d| d.from.as_str()).collect();
        assert_eq!(
            from,
            [
                "sysctl:hw.perflevel0.physicalcpu",
                "sysctl:hw.perflevel0.l2cachesize",
                "sysctl:hw.perflevel1.l1dcachesize"
            ]
        );
    }

    #[test]
    fn absurd_perflevel_count_falls_back_to_one_cluster() {
        let cpu = collect_sys(&with(&[("hw.nperflevels", Some(Int(99)))]));
        assert_eq!(cpu.clusters.len(), 1);
        assert_eq!(cpu.diagnostics.len(), 1);
    }

    #[test]
    fn control_characters_are_rejected_not_printed() {
        let cpu = collect_sys(&with(&[
            (
                "machdep.cpu.brand_string",
                Some(Str("Apple \u{1b}]0;pwned\u{7}M5".into())),
            ),
            ("hw.perflevel0.name", Some(Str("S\tuper".into()))),
        ]));
        assert_eq!(cpu.identity.name, None);
        assert_eq!(cpu.clusters[0].name, None);
        let from: Vec<&str> = cpu.diagnostics.iter().map(|d| d.from.as_str()).collect();
        assert_eq!(
            from,
            [
                "sysctl:machdep.cpu.brand_string",
                "sysctl:hw.perflevel0.name"
            ]
        );
    }

    #[test]
    fn an_empty_machine_is_not_identified() {
        assert!(!collect_sys(&BTreeMap::<String, SysctlValue>::new()).is_identified());
    }

    const M5_P: &str = "60f51300160300004089180016030000e07a1d002a030000e0df210048030000e044260052030000a0072b006103000000e02e0084030000a05a3200a203000060a63500b603000040c33800ca030000a0243b00e803000020573d000604000080b83f0024040000c0d1400024040000802f410024040000e019420024040000e0904300240400000062430047040000801d440047040000";
    const M5_E: &str = "e0d40e00160300000094110016030000802b18002a03000040651e003e03000080e3230061030000e032290089030000203a2d00ca03000040822e00ca030000";

    fn m5_tables() -> BTreeMap<String, Vec<u8>> {
        let hex = crate::source::ioreg::decode_hex;
        BTreeMap::from([
            ("pmgr:voltage-states5-sram".to_string(), hex(M5_P).unwrap()),
            ("pmgr:voltage-states1-sram".to_string(), hex(M5_E).unwrap()),
        ])
    }

    #[test]
    fn frequency_tables_decode_both_units() {
        let hex = crate::source::ioreg::decode_hex;
        assert_eq!(
            max_frequency(&hex(M5_P).unwrap()),
            Some(4_464_000_000),
            "M5 stores kHz, unsorted"
        );
        assert_eq!(max_frequency(&hex(M5_E).unwrap()), Some(3_048_000_000));
        // M1-style hertz: 600 MHz and 3.204 GHz.
        let hz: Vec<u8> = [600_000_000u32, 3_204_000_000]
            .iter()
            .flat_map(|f| [f.to_le_bytes(), 0u32.to_le_bytes()].concat())
            .collect();
        assert_eq!(max_frequency(&hz), Some(3_204_000_000));
    }

    #[test]
    fn odd_tables_are_rejected() {
        assert_eq!(max_frequency(&[]), None);
        assert_eq!(max_frequency(&[0; 16]), None);
        assert_eq!(max_frequency(&[1, 2, 3]), None, "shorter than one entry");
    }

    #[test]
    fn perflevels_get_their_max_clock() {
        let cpu = collect(&with(&[]), &m5_tables());
        let max: Vec<Option<Hertz>> = cpu.clusters.iter().map(|c| value(&c.clock.max)).collect();
        assert_eq!(
            max,
            [Some(Hertz(4_464_000_000)), Some(Hertz(3_048_000_000))]
        );
        assert_eq!(
            cpu.clusters[0].clock.max.as_ref().unwrap().origin,
            Origin::Detected("ioreg:pmgr:voltage-states5-sram".into())
        );
    }

    #[test]
    fn clock_tables_need_exactly_two_perflevels() {
        let one = collect(&with(&[("hw.nperflevels", Some(Int(1)))]), &m5_tables());
        assert!(one.clusters.iter().all(|c| c.clock.is_empty()));
    }

    #[test]
    fn a_bad_table_is_a_diagnostic() {
        let tables = BTreeMap::from([("pmgr:voltage-states5-sram".to_string(), vec![0u8; 16])]);
        let cpu = collect(&with(&[]), &tables);
        assert!(cpu.clusters[0].clock.is_empty());
        assert!(
            cpu.diagnostics
                .iter()
                .any(|d| d.from == "ioreg:pmgr:voltage-states5-sram"),
            "{:?}",
            cpu.diagnostics
        );
    }

    #[test]
    fn rosetta_reports_the_real_chip() {
        let cpu = crate::test_support::fixture("apple-m5-rosetta");
        assert_eq!(
            cpu.identity.translated,
            Some(Fact::detected(true, "sysctl:sysctl.proc_translated"))
        );
        assert_eq!(value(&cpu.identity.name).as_deref(), Some("Apple M5"));
        assert_eq!(value(&cpu.identity.arch).as_deref(), Some("arm64"));
        assert_eq!(
            cpu.identity.x86_family, None,
            "Rosetta's fake family 6 is ignored"
        );
        assert_eq!(
            value(&cpu.clusters[0].clock.max),
            Some(Hertz(4_464_000_000)),
            "not the fake 2.4 GHz"
        );
        assert!(
            cpu.features.raw.iter().all(|f| !f.contains("SSE")),
            "no emulated x86 features"
        );
    }

    #[test]
    fn native_runs_are_not_marked_translated() {
        assert_eq!(
            crate::test_support::fixture("apple-m5").identity.translated,
            None
        );
    }

    #[test]
    fn intel_mac_identity_and_topology() {
        let cpu = crate::test_support::fixture("intel-mac-i9-9880h");
        let id = &cpu.identity;
        assert_eq!(
            value(&id.name).as_deref(),
            Some("Intel(R) Core(TM) i9-9880H CPU @ 2.30GHz")
        );
        assert_eq!(value(&id.vendor).as_deref(), Some("Intel"));
        assert_eq!(value(&id.arch).as_deref(), Some("x86_64"));
        assert_eq!(
            (
                value(&id.x86_family),
                value(&id.x86_model),
                value(&id.x86_stepping)
            ),
            (Some(6), Some(158), Some(13))
        );
        assert_eq!(value(&id.microcode).as_deref(), Some("0xf8"));
        let t = &cpu.topology;
        assert_eq!(
            (
                value(&t.physical_cores),
                value(&t.logical_cpus),
                value(&t.smt_per_core)
            ),
            (Some(8), Some(16), Some(2))
        );
    }

    /// (level, size, shared_by, cores, instances)
    type Shape = (u8, Option<Bytes>, Option<u32>, Option<u32>, Option<u32>);

    #[test]
    fn intel_mac_caches_use_cacheconfig_for_sharing() {
        let cpu = crate::test_support::fixture("intel-mac-i9-9880h");
        assert_eq!(cpu.clusters.len(), 1);
        let shapes: Vec<Shape> = cpu.clusters[0]
            .caches
            .iter()
            .map(|k| {
                (
                    k.level,
                    value(&k.size),
                    value(&k.shared_by),
                    value(&k.cores),
                    value(&k.instances),
                )
            })
            .collect();
        assert_eq!(
            shapes,
            [
                (1, Some(Bytes(32 << 10)), Some(2), Some(1), Some(8)),
                (1, Some(Bytes(32 << 10)), Some(2), Some(1), Some(8)),
                (2, Some(Bytes(256 << 10)), Some(2), Some(1), Some(8)),
                (3, Some(Bytes(16 << 20)), Some(16), Some(8), Some(1)),
            ]
        );
    }

    #[test]
    fn intel_mac_hides_a_max_equal_to_base() {
        let cpu = crate::test_support::fixture("intel-mac-i9-9880h");
        let clock = &cpu.clusters[0].clock;
        assert_eq!(value(&clock.base), Some(Hertz(2_300_000_000)));
        assert_eq!(
            clock.max, None,
            "hw.cpufrequency_max is the nominal clock, not turbo"
        );
    }

    #[test]
    fn intel_mac_features_are_x86() {
        let cpu = crate::test_support::fixture("intel-mac-i9-9880h");
        let names: Vec<&str> = cpu
            .features
            .groups
            .iter()
            .flat_map(|g| g.features.iter().map(|f| f.name.as_str()))
            .collect();
        assert_eq!(
            names,
            [
                "SSE4.2", "AVX", "AVX2", "FMA", "AES", "PCLMUL", "RDRAND", "RDSEED", "VT-x",
                "SMEP", "SMAP"
            ]
        );
    }

    #[test]
    fn truncated_tables_and_placeholders_are_rejected() {
        let hex = crate::source::ioreg::decode_hex;
        let mut cut = hex(M5_P).unwrap();
        cut.truncate(cut.len() - 4);
        assert_eq!(
            max_frequency(&cut),
            None,
            "a partial entry means the table is damaged"
        );
        let placeholder: Vec<u8> = [1_000_000_000u32, u32::MAX]
            .iter()
            .flat_map(|f| [f.to_le_bytes(), 0u32.to_le_bytes()].concat())
            .collect();
        assert_eq!(
            max_frequency(&placeholder),
            Some(1_000_000_000),
            "0xffffffff is Apple's placeholder, not 4.29 GHz"
        );
    }

    #[test]
    fn clocks_need_an_efficiency_second_core_type() {
        let cpu = collect(
            &with(&[("hw.perflevel1.name", Some(Str("Performance".into())))]),
            &m5_tables(),
        );
        assert!(
            cpu.clusters.iter().all(|c| c.clock.is_empty()),
            "the 5/1 table mapping is only known for P + E chips"
        );
    }

    #[test]
    fn a_slower_performance_table_is_rejected() {
        let hex = crate::source::ioreg::decode_hex;
        let swapped = BTreeMap::from([
            ("pmgr:voltage-states5-sram".to_string(), hex(M5_E).unwrap()),
            ("pmgr:voltage-states1-sram".to_string(), hex(M5_P).unwrap()),
        ]);
        let cpu = collect(&with(&[]), &swapped);
        assert!(cpu.clusters.iter().all(|c| c.clock.is_empty()));
        assert!(
            cpu.diagnostics.iter().any(|d| d.message.contains("slower")),
            "{:?}",
            cpu.diagnostics
        );
    }
}
