//! macOS collector. Apple Silicon reports each core type as a "perflevel" (`hw.perflevel0` is the
//! fastest); each becomes one [`Cluster`]. Compiled on every OS so fixtures test it anywhere.

use crate::db::Arch;
use crate::model::{
    Cache, CacheKind, Clocks, Cluster, CoreKind, Cpu, Diagnostic, F, Fact, Identity, Topology,
};
use crate::source::Sysctl;
use crate::units::Bytes;

use super::{features, plausible_cache_size, ratio};

/// More perflevels than this is garbage, not a real chip.
const MAX_PERFLEVELS: u32 = 8;

pub fn collect(sys: &dyn Sysctl) -> Cpu {
    let mut r = Reader {
        sys,
        diagnostics: Vec::new(),
    };
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
    Identity {
        name,
        vendor,
        arch,
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
        return (0..levels).map(|i| perflevel(r, i, levels)).collect();
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
            instances: ratio(&threads, shared_by),
        });
    }
    Cluster {
        kind,
        name,
        cores,
        threads,
        clock: Clocks::default(),
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
        let cpu = collect(&with(&[]));
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
        let cpu = collect(&with(&[]));
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
        let cpu = collect(&with(&[
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
        let cpu = collect(&with(&[]));
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
        assert_eq!(collect(&with(&[])).features.raw, vec!["FEAT_AES"]);
    }

    #[test]
    fn without_perflevels_there_is_one_cluster_and_no_guessed_caches() {
        let cpu = collect(&with(&[("hw.nperflevels", None)]));
        assert_eq!(cpu.clusters.len(), 1);
        assert_eq!(cpu.clusters[0].kind, CoreKind::Uniform);
        assert!(
            cpu.clusters[0].caches.is_empty(),
            "top-level hw.l2cachesize describes E-cores only"
        );
    }

    #[test]
    fn implausible_values_are_dropped_with_a_diagnostic() {
        let cpu = collect(&with(&[
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
        let cpu = collect(&with(&[("hw.nperflevels", Some(Int(99)))]));
        assert_eq!(cpu.clusters.len(), 1);
        assert_eq!(cpu.diagnostics.len(), 1);
    }

    #[test]
    fn control_characters_are_rejected_not_printed() {
        let cpu = collect(&with(&[
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
        assert!(!collect(&BTreeMap::<String, SysctlValue>::new()).is_identified());
    }
}
