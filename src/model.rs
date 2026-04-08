//! The one data model every collector fills and every renderer reads.

use serde::Serialize;
use serde::ser::{SerializeMap, Serializer};

use crate::units::{Bytes, Hertz};

/// Where a value came from.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Origin {
    /// Read from this machine; holds the exact key or path, e.g. `sysctl:hw.ncpu`.
    Detected(String),
    /// Computed from other facts, e.g. SMT = logical / physical.
    Derived,
    /// Filled from a built-in table, e.g. `apple-chips@2026-09`.
    Database(&'static str),
}

/// A value plus where it came from.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Fact<T> {
    pub value: T,
    pub origin: Origin,
}

/// A value that may be unknown. `None` hides the row; it is never guessed or printed.
pub type F<T> = Option<Fact<T>>;

impl<T> Fact<T> {
    pub fn detected(value: T, from: impl Into<String>) -> Self {
        Fact {
            value,
            origin: Origin::Detected(from.into()),
        }
    }

    pub fn derived(value: T) -> Self {
        Fact {
            value,
            origin: Origin::Derived,
        }
    }
}

impl<T: Serialize> Serialize for Fact<T> {
    fn serialize<S: Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        let (origin, from) = match &self.origin {
            Origin::Detected(key) => ("detected", Some(key.as_str())),
            Origin::Derived => ("derived", None),
            Origin::Database(table) => ("database", Some(*table)),
        };
        let mut map = s.serialize_map(Some(if from.is_some() { 3 } else { 2 }))?;
        map.serialize_entry("value", &self.value)?;
        map.serialize_entry("origin", origin)?;
        if let Some(from) = from {
            map.serialize_entry("from", from)?;
        }
        map.end()
    }
}

#[derive(Debug, Clone, Default, PartialEq, Serialize)]
pub struct Cpu {
    pub identity: Identity,
    pub topology: Topology,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub clusters: Vec<Cluster>,
    /// Caches that span core types, e.g. an Intel hybrid L3 or Apple's SLC.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub shared_caches: Vec<Cache>,
    #[serde(skip_serializing_if = "Features::is_empty")]
    pub features: Features,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub diagnostics: Vec<Diagnostic>,
}

impl Cpu {
    /// Whether enough was learned to be worth printing: a name or a CPU count.
    pub fn is_identified(&self) -> bool {
        self.identity.name.is_some() || self.topology.logical_cpus.is_some()
    }
}

#[derive(Debug, Clone, Default, PartialEq, Serialize)]
pub struct Identity {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub vendor: F<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub name: F<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub arch: F<String>,
    /// e.g. `ARMv8.6-A`, `x86-64-v4`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub isa_level: F<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub x86_family: F<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub x86_model: F<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub x86_stepping: F<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub arm_midr: F<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub microcode: F<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub hypervisor: F<String>,
    /// Running under Rosetta 2.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub translated: F<bool>,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize)]
pub struct Topology {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub sockets: F<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub physical_cores: F<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub logical_cpus: F<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub smt_per_core: F<u32>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub numa_nodes: Vec<NumaNode>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct NumaNode {
    pub id: u32,
    pub cpus: Vec<u32>,
}

/// A group of cores of one type across the whole system, not a physical cluster.
/// Physical sharing is expressed by [`Cache::shared_by`] and [`Cache::instances`].
#[derive(Debug, Clone, Default, PartialEq, Serialize)]
pub struct Cluster {
    pub kind: CoreKind,
    /// The OS's own name for this core type, e.g. `Super` on an Apple M5.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub name: F<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub cores: F<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub threads: F<u32>,
    #[serde(skip_serializing_if = "Clocks::is_empty")]
    pub clock: Clocks,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub caches: Vec<Cache>,
}

impl Cluster {
    /// Column heading: the OS's name if it gave one, else the kind.
    pub fn label(&self) -> &str {
        self.name
            .as_ref()
            .map_or(self.kind.label(), |n| n.value.as_str())
    }
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum CoreKind {
    Performance,
    Efficiency,
    #[default]
    Uniform,
}

impl CoreKind {
    pub fn label(self) -> &'static str {
        match self {
            CoreKind::Performance => "Performance",
            CoreKind::Efficiency => "Efficiency",
            CoreKind::Uniform => "Cores",
        }
    }
}

#[derive(Debug, Clone, Default, PartialEq, Serialize)]
pub struct Clocks {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub base: F<Hertz>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub max: F<Hertz>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub current: F<Hertz>,
}

impl Clocks {
    pub fn is_empty(&self) -> bool {
        self.base.is_none() && self.max.is_none() && self.current.is_none()
    }
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Cache {
    pub level: u8,
    pub kind: CacheKind,
    /// Size of one instance.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub size: F<Bytes>,
    /// Logical CPUs sharing one instance.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub shared_by: F<u32>,
    /// How many instances exist in this cluster.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub instances: F<u32>,
}

/// Declaration order is display order: L1i before L1d.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum CacheKind {
    Instruction,
    Data,
    Unified,
}

impl CacheKind {
    /// Suffix after the level in labels: `L1i`, `L1d`, `L2`.
    pub fn suffix(self) -> &'static str {
        match self {
            CacheKind::Instruction => "i",
            CacheKind::Data => "d",
            CacheKind::Unified => "",
        }
    }
}

#[derive(Debug, Clone, Default, PartialEq, Serialize)]
pub struct Features {
    pub groups: Vec<FeatureGroupEntry>,
    /// Every enabled flag exactly as the OS reported it, including ones the table doesn't show.
    pub raw: Vec<String>,
}

impl Features {
    pub fn is_empty(&self) -> bool {
        self.groups.is_empty() && self.raw.is_empty()
    }
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct FeatureGroupEntry {
    pub group: FeatureGroup,
    pub features: Vec<Feature>,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Feature {
    pub raw: String,
    pub name: String,
    /// Display family: members of one family are shown together, e.g. `AVX-512 (F/BW)`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub family: Option<String>,
}

/// Declaration order is display order.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum FeatureGroup {
    Simd,
    Crypto,
    Virtualization,
    Security,
    Other,
}

impl FeatureGroup {
    pub fn label(self) -> &'static str {
        match self {
            FeatureGroup::Simd => "SIMD",
            FeatureGroup::Crypto => "Crypto",
            FeatureGroup::Virtualization => "Virtualization",
            FeatureGroup::Security => "Security",
            FeatureGroup::Other => "Other",
        }
    }
}

/// A value that existed but was rejected, e.g. a 0-byte cache. Shown by `--explain` (Plan 3).
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Diagnostic {
    pub from: String,
    pub message: String,
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    #[test]
    fn detected_fact_serializes_with_its_source() {
        let fact = Fact::detected(4u32, "sysctl:hw.ncpu");
        assert_eq!(
            serde_json::to_value(&fact).unwrap(),
            json!({"value": 4, "origin": "detected", "from": "sysctl:hw.ncpu"})
        );
    }

    #[test]
    fn derived_fact_has_no_from() {
        assert_eq!(
            serde_json::to_value(Fact::derived("Apple")).unwrap(),
            json!({"value": "Apple", "origin": "derived"})
        );
    }

    #[test]
    fn database_fact_names_its_table() {
        let fact = Fact {
            value: 1u32,
            origin: Origin::Database("apple-chips@2026-09"),
        };
        assert_eq!(
            serde_json::to_value(&fact).unwrap(),
            json!({"value": 1, "origin": "database", "from": "apple-chips@2026-09"})
        );
    }

    #[test]
    fn unknown_fields_are_omitted_not_null() {
        let mut cpu = Cpu::default();
        cpu.identity.name = Some(Fact::detected(
            "Apple M5".to_string(),
            "sysctl:machdep.cpu.brand_string",
        ));
        assert_eq!(
            serde_json::to_value(&cpu).unwrap(),
            json!({
                "identity": {"name": {"value": "Apple M5", "origin": "detected", "from": "sysctl:machdep.cpu.brand_string"}},
                "topology": {}
            })
        );
    }

    #[test]
    fn identified_needs_a_name_or_a_cpu_count() {
        let mut cpu = Cpu::default();
        assert!(!cpu.is_identified());
        cpu.topology.logical_cpus = Some(Fact::derived(8));
        assert!(cpu.is_identified());
    }

    #[test]
    fn cluster_label_prefers_the_os_name() {
        let mut cluster = Cluster {
            kind: CoreKind::Performance,
            ..Cluster::default()
        };
        assert_eq!(cluster.label(), "Performance");
        cluster.name = Some(Fact::detected(
            "Super".to_string(),
            "sysctl:hw.perflevel0.name",
        ));
        assert_eq!(cluster.label(), "Super");
        assert_eq!(Cluster::default().label(), "Cores");
    }
}
