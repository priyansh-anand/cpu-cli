//! Linux collector: `/proc/cpuinfo`, `/sys/devices/system/{cpu,node}` and two DMI strings.
//! Compiled on every OS, so Linux fixtures are tested on any machine.

use std::cmp::Reverse;
use std::collections::{BTreeMap, BTreeSet};

use super::sysfs::{parse_cpu_list, parse_cpuinfo, parse_size};
use super::{count, features, plausible_cache_size, ratio};
use crate::db::{self, Arch};
use crate::model::{
    Cache, CacheKind, Clocks, Cluster, CoreKind, Cpu, Diagnostic, F, Fact, Identity, NumaNode,
    Origin, Topology,
};
use crate::source::Fs;
use crate::units::{Bytes, Hertz};

const CPU_DIR: &str = "/sys/devices/system/cpu";
const NODE_DIR: &str = "/sys/devices/system/node";
const CPUINFO: &str = "/proc/cpuinfo";
const ARCH: &str = "/proc/sys/kernel/arch";
const DMI_VENDOR: &str = "/sys/class/dmi/id/sys_vendor";
const DMI_PRODUCT: &str = "/sys/class/dmi/id/product_name";

/// DMI names meaning "virtual machine". Only used on ARM, which has no hypervisor CPU flag.
const VIRTUAL_PLATFORMS: &[&str] = &[
    "KVM",
    "QEMU",
    "VMware",
    "VirtualBox",
    "Virtual Machine",
    "Xen",
    "HVM domU",
    "Google Compute Engine",
    "Apple Virtualization",
    "Parallels",
    "BHYVE",
    "OpenStack",
    "Bochs",
];

/// One `/proc/cpuinfo` processor block: field name to value.
type Block = BTreeMap<String, String>;

pub fn collect(fs: &dyn Fs) -> Cpu {
    let mut r = Reader {
        fs,
        diagnostics: Vec::new(),
    };
    let info = r.cpuinfo();
    let cpus = r.online_cpus();
    let topo = cpu_topology(&mut r, &cpus);
    let identity = identity(&mut r, &info);
    let topology = topology(&mut r, &cpus, &topo);
    let (clusters, shared_caches) = clusters(&mut r, &cpus, &info, &topo);
    let arch = if is_x86(&info) { Arch::X86 } else { Arch::Arm };
    let features = features::group(flags(&info), arch);
    Cpu {
        identity,
        topology,
        clusters,
        shared_caches,
        features,
        diagnostics: r.diagnostics,
    }
}

/// Reads files, recording a [`Diagnostic`] for values that exist but make no sense. A missing
/// file is normal (older kernels, containers, VMs) and is not a diagnostic.
struct Reader<'a> {
    fs: &'a dyn Fs,
    diagnostics: Vec<Diagnostic>,
}

impl Reader<'_> {
    /// A file's trimmed contents; `None` when missing or empty.
    fn text(&mut self, path: &str) -> Option<String> {
        let text = self.fs.read(path)?;
        let text = text.trim();
        if text.is_empty() {
            return None;
        }
        // Snapshots can come from anyone; a control character could drive the reader's terminal.
        if text.chars().any(char::is_control) {
            self.reject(path, "contains control characters");
            return None;
        }
        Some(text.to_string())
    }

    fn number<T: std::str::FromStr>(&mut self, path: &str) -> Option<T> {
        let text = self.text(path)?;
        match text.parse() {
            Ok(n) => Some(n),
            Err(_) => {
                self.reject(path, "not a number");
                None
            }
        }
    }

    fn cpu_list(&mut self, path: &str) -> Option<Vec<u32>> {
        let text = self.text(path)?;
        match parse_cpu_list(&text) {
            Some(list) => Some(list),
            None => {
                self.reject(path, "not a CPU list");
                None
            }
        }
    }

    fn size(&mut self, path: &str) -> F<Bytes> {
        let text = self.text(path)?;
        match parse_size(&text).filter(|n| plausible_cache_size(*n)) {
            Some(n) => Some(Fact::detected(Bytes(n), path)),
            None => {
                self.reject(path, "implausible cache size");
                None
            }
        }
    }

    fn cpuinfo(&self) -> Vec<Block> {
        self.fs
            .read(CPUINFO)
            .map(|t| parse_cpuinfo(&t))
            .unwrap_or_default()
    }

    /// Online CPU numbers, or every `cpuN` directory when `online` is missing or unreadable.
    fn online_cpus(&mut self) -> Vec<u32> {
        if let Some(cpus) = self.cpu_list(&format!("{CPU_DIR}/online")) {
            return cpus;
        }
        let mut cpus: Vec<u32> = self
            .fs
            .list(CPU_DIR)
            .iter()
            .filter_map(|name| name.strip_prefix("cpu")?.parse().ok())
            .collect();
        cpus.sort_unstable();
        cpus
    }

    fn reject(&mut self, from: &str, message: &str) {
        self.diagnostics.push(Diagnostic {
            from: from.to_string(),
            message: message.to_string(),
        });
    }
}

/// What sysfs says about one logical CPU's place in the machine.
struct CpuTopo {
    package: Option<i64>,
    siblings: Option<Vec<u32>>,
}

fn cpu_topology(r: &mut Reader, cpus: &[u32]) -> BTreeMap<u32, CpuTopo> {
    cpus.iter()
        .map(|&cpu| {
            let base = format!("{CPU_DIR}/cpu{cpu}/topology");
            let topo = CpuTopo {
                package: r.number(&format!("{base}/physical_package_id")),
                // core_cpus_list is the newer name; some kernels only have one of them.
                siblings: r
                    .cpu_list(&format!("{base}/thread_siblings_list"))
                    .or_else(|| r.cpu_list(&format!("{base}/core_cpus_list"))),
            };
            (cpu, topo)
        })
        .collect()
}

fn topology(r: &mut Reader, cpus: &[u32], topo: &BTreeMap<u32, CpuTopo>) -> Topology {
    let logical_cpus = count(cpus.len());
    // A core is a set of hardware threads; count distinct sibling sets, but only when every CPU
    // reported one, so a partial read never undercounts.
    let complete = !topo.is_empty() && topo.values().all(|t| t.siblings.is_some());
    let cores: BTreeSet<&Vec<u32>> = topo.values().filter_map(|t| t.siblings.as_ref()).collect();
    let physical_cores = if complete { count(cores.len()) } else { None };
    let packages: BTreeSet<i64> = topo
        .values()
        .filter_map(|t| t.package)
        .filter(|p| *p >= 0)
        .collect();
    Topology {
        sockets: count(packages.len()),
        smt_per_core: ratio(&logical_cpus, &physical_cores),
        physical_cores,
        logical_cpus,
        numa_nodes: numa(r),
    }
}

fn numa(r: &mut Reader) -> Vec<NumaNode> {
    let mut nodes: Vec<NumaNode> =
        r.fs.list(NODE_DIR)
            .into_iter()
            .filter_map(|name| {
                let id: u32 = name.strip_prefix("node")?.parse().ok()?;
                let cpus = r.cpu_list(&format!("{NODE_DIR}/{name}/cpulist"))?;
                (!cpus.is_empty()).then_some(NumaNode { id, cpus })
            })
            .collect();
    nodes.sort_by_key(|n| n.id);
    nodes
}

fn identity(r: &mut Reader, info: &[Block]) -> Identity {
    let arch = r
        .text(ARCH)
        .map(|a| Fact::detected(a, ARCH))
        .or_else(|| derived_arch(info));
    let hypervisor = hypervisor(r, info);
    let mut id = if is_x86(info) {
        x86_identity(r, info)
    } else {
        arm_identity(r, info)
    };
    id.arch = arch;
    id.hypervisor = hypervisor;
    id
}

fn is_x86(info: &[Block]) -> bool {
    info.first().is_some_and(|b| b.contains_key("vendor_id"))
}

fn x86_identity(r: &mut Reader, info: &[Block]) -> Identity {
    Identity {
        vendor: field(r, info, "vendor_id").map(|f| Fact {
            value: x86_vendor(&f.value),
            origin: f.origin,
        }),
        name: field(r, info, "model name"),
        x86_family: number_field(r, info, "cpu family"),
        x86_model: number_field(r, info, "model"),
        x86_stepping: number_field(r, info, "stepping"),
        microcode: field(r, info, "microcode"),
        ..Identity::default()
    }
}

fn x86_vendor(vendor_id: &str) -> String {
    match vendor_id {
        "GenuineIntel" => "Intel",
        "AuthenticAMD" => "AMD",
        "HygonGenuine" => "Hygon",
        "CentaurHauls" => "Centaur",
        other => other,
    }
    .to_string()
}

fn arm_identity(r: &mut Reader, info: &[Block]) -> Identity {
    let Some(implementer) = hex_field(r, info, "CPU implementer") else {
        return Identity::default();
    };
    let part = hex_field(r, info, "CPU part");
    let midr_path = format!("{CPU_DIR}/cpu0/regs/identification/midr_el1");
    let arm_midr = r
        .text(&midr_path)
        .and_then(|t| parse_midr(&t))
        .map(|m| Fact::detected(m, midr_path));
    Identity {
        vendor: db::arm_implementer(implementer).map(database),
        name: field(r, info, "model name").or_else(|| {
            part.and_then(|p| db::arm_part(implementer, p))
                .map(database)
        }),
        arm_midr,
        ..Identity::default()
    }
}

/// `midr_el1` is a 64-bit register; MIDR is its low 32 bits.
fn parse_midr(text: &str) -> Option<u32> {
    let register = u64::from_str_radix(text.trim_start_matches("0x"), 16).ok()?;
    u32::try_from(register & 0xffff_ffff).ok()
}

fn database(value: &str) -> Fact<String> {
    Fact {
        value: value.to_string(),
        origin: Origin::Database(db::ARM_MIDR_STAMP),
    }
}

fn derived_arch(info: &[Block]) -> F<String> {
    let first = info.first()?;
    if first
        .get("flags")
        .is_some_and(|f| f.split_whitespace().any(|x| x == "lm"))
    {
        return Some(Fact::derived("x86_64".to_string()));
    }
    (first.get("CPU architecture").map(String::as_str) == Some("8"))
        .then(|| Fact::derived("aarch64".to_string()))
}

/// x86 trusts the `hypervisor` CPU flag; ARM has none, so it recognises virtual-platform DMI
/// names. The value shown is the DMI product name.
fn hypervisor(r: &mut Reader, info: &[Block]) -> F<String> {
    let vendor = r.text(DMI_VENDOR);
    let product = r.text(DMI_PRODUCT);
    let virtual_machine = if is_x86(info) {
        info.first()
            .and_then(|b| b.get("flags"))
            .is_some_and(|f| f.split_whitespace().any(|x| x == "hypervisor"))
    } else {
        [&vendor, &product].iter().any(|name| {
            name.as_deref()
                .is_some_and(|n| VIRTUAL_PLATFORMS.iter().any(|m| n.contains(m)))
        })
    };
    if !virtual_machine {
        return None;
    }
    match (product, vendor) {
        (Some(product), _) => Some(Fact::detected(product, DMI_PRODUCT)),
        (None, Some(vendor)) => Some(Fact::detected(vendor, DMI_VENDOR)),
        (None, None) => Some(Fact::detected(
            "unknown".to_string(),
            format!("{CPUINFO}:flags"),
        )),
    }
}

/// A field of the first cpuinfo block.
fn field(r: &mut Reader, info: &[Block], key: &str) -> F<String> {
    let value = info.first()?.get(key)?.clone();
    let from = format!("{CPUINFO}:{key}");
    if value.is_empty() {
        return None;
    }
    if value.chars().any(char::is_control) {
        r.reject(&from, "contains control characters");
        return None;
    }
    Some(Fact::detected(value, from))
}

fn number_field(r: &mut Reader, info: &[Block], key: &str) -> F<u32> {
    let f = field(r, info, key)?;
    match f.value.parse() {
        Ok(n) => Some(Fact {
            value: n,
            origin: f.origin,
        }),
        Err(_) => {
            r.reject(&format!("{CPUINFO}:{key}"), "not a number");
            None
        }
    }
}

fn hex_field(r: &mut Reader, info: &[Block], key: &str) -> Option<u32> {
    let f = field(r, info, key)?;
    let parsed = parse_hex(&f.value);
    if parsed.is_none() {
        r.reject(&format!("{CPUINFO}:{key}"), "not a hex number");
    }
    parsed
}

fn parse_hex(text: &str) -> Option<u32> {
    u32::from_str_radix(text.trim().trim_start_matches("0x"), 16).ok()
}

/// Feature flags of the first processor block (`flags` on x86, `Features` on ARM), sorted.
fn flags(info: &[Block]) -> Vec<String> {
    let Some(first) = info.first() else {
        return Vec::new();
    };
    let mut flags: Vec<String> = first
        .get("flags")
        .or_else(|| first.get("Features"))
        .map(|f| f.split_whitespace().map(str::to_string).collect())
        .unwrap_or_default();
    flags.sort();
    flags.dedup();
    flags
}

/// One core type's CPUs, before they become a [`Cluster`].
struct Group {
    kind: CoreKind,
    name: F<String>,
    cpus: Vec<u32>,
}

fn clusters(
    r: &mut Reader,
    cpus: &[u32],
    info: &[Block],
    topo: &BTreeMap<u32, CpuTopo>,
) -> (Vec<Cluster>, Vec<Cache>) {
    if cpus.is_empty() {
        return (Vec::new(), Vec::new());
    }
    let groups = groups(r, cpus, info);
    let (caches, shared) = place(cache_instances(r, cpus, topo), &groups);
    let clusters = groups
        .into_iter()
        .zip(caches)
        .map(|(group, caches)| {
            let complete = group
                .cpus
                .iter()
                .all(|c| topo.get(c).is_some_and(|t| t.siblings.is_some()));
            let cores: BTreeSet<&Vec<u32>> = group
                .cpus
                .iter()
                .filter_map(|c| topo.get(c)?.siblings.as_ref())
                .collect();
            Cluster {
                kind: group.kind,
                name: group.name,
                cores: if complete { count(cores.len()) } else { None },
                threads: count(group.cpus.len()),
                clock: clocks(r, &group.cpus),
                caches,
            }
        })
        .collect();
    (clusters, shared)
}

fn pmu_cpus(r: &mut Reader, pmu: &str, online: &BTreeSet<u32>) -> Vec<u32> {
    r.cpu_list(&format!("/sys/devices/{pmu}/cpus"))
        .unwrap_or_default()
        .into_iter()
        .filter(|c| online.contains(c))
        .collect()
}

fn groups(r: &mut Reader, cpus: &[u32], info: &[Block]) -> Vec<Group> {
    let online: BTreeSet<u32> = cpus.iter().copied().collect();
    // Intel hybrid: the kernel lists each core type's CPUs under its PMU.
    let (performance, efficiency) = (
        pmu_cpus(r, "cpu_core", &online),
        pmu_cpus(r, "cpu_atom", &online),
    );
    if !performance.is_empty() && !efficiency.is_empty() {
        return vec![
            Group {
                kind: CoreKind::Performance,
                name: None,
                cpus: performance,
            },
            Group {
                kind: CoreKind::Efficiency,
                name: None,
                cpus: efficiency,
            },
        ];
    }
    // ARM big.LITTLE: one group per (capacity, core part), highest capacity first.
    let implementer = info
        .first()
        .and_then(|b| b.get("CPU implementer"))
        .and_then(|v| parse_hex(v));
    let mut by_type: BTreeMap<(Reverse<u32>, u32), Vec<u32>> = BTreeMap::new();
    for &cpu in cpus {
        let capacity = r
            .number::<u32>(&format!("{CPU_DIR}/cpu{cpu}/cpu_capacity"))
            .unwrap_or(0);
        let part = block_for(info, cpu)
            .and_then(|b| b.get("CPU part"))
            .and_then(|p| parse_hex(p))
            .unwrap_or(0);
        by_type
            .entry((Reverse(capacity), part))
            .or_default()
            .push(cpu);
    }
    if by_type.len() <= 1 {
        return vec![Group {
            kind: CoreKind::Uniform,
            name: None,
            cpus: cpus.to_vec(),
        }];
    }
    let last = by_type.len() - 1;
    by_type
        .into_iter()
        .enumerate()
        .map(|(i, ((_, part), cpus))| Group {
            kind: if i == last {
                CoreKind::Efficiency
            } else {
                CoreKind::Performance
            },
            name: implementer
                .and_then(|imp| db::arm_part(imp, part))
                .map(database),
            cpus,
        })
        .collect()
}

fn block_for(info: &[Block], cpu: u32) -> Option<&Block> {
    info.iter()
        .find(|b| b.get("processor").and_then(|p| p.parse::<u32>().ok()) == Some(cpu))
}

/// One cache instance: a level and type, and the online CPUs sharing it.
struct Instance {
    level: u8,
    kind: CacheKind,
    size: F<Bytes>,
    cpus: Vec<u32>,
    /// Physical cores among `cpus`, when every one of them reported its siblings.
    cores: Option<usize>,
}

fn cache_instances(r: &mut Reader, cpus: &[u32], topo: &BTreeMap<u32, CpuTopo>) -> Vec<Instance> {
    let online: BTreeSet<u32> = cpus.iter().copied().collect();
    let mut seen = BTreeSet::new();
    let mut out = Vec::new();
    for &cpu in cpus {
        let dir = format!("{CPU_DIR}/cpu{cpu}/cache");
        for index in
            r.fs.list(&dir)
                .into_iter()
                .filter(|n| n.starts_with("index"))
        {
            let base = format!("{dir}/{index}");
            let Some(level) = r
                .number::<u8>(&format!("{base}/level"))
                .filter(|l| (1..=4).contains(l))
            else {
                continue;
            };
            let kind = match r.text(&format!("{base}/type")).as_deref() {
                Some("Data") => CacheKind::Data,
                Some("Instruction") => CacheKind::Instruction,
                Some("Unified") => CacheKind::Unified,
                _ => continue,
            };
            // Only online CPUs share anything; offline ones would skew the counts.
            let mut sharers: Vec<u32> = r
                .cpu_list(&format!("{base}/shared_cpu_list"))
                .unwrap_or_default()
                .into_iter()
                .filter(|c| online.contains(c))
                .collect();
            if sharers.is_empty() {
                sharers.push(cpu);
            }
            if !seen.insert((level, kind, sharers.clone())) {
                continue;
            }
            let size = r.size(&format!("{base}/size"));
            let cores = sharer_cores(&sharers, topo);
            out.push(Instance {
                level,
                kind,
                size,
                cpus: sharers,
                cores,
            });
        }
    }
    out
}

type Buckets = BTreeMap<(u8, CacheKind), Vec<Instance>>;

/// Gives each cache instance to the group owning all its CPUs. An instance whose CPUs span
/// several groups (an Intel hybrid L3) becomes a shared cache.
fn place(instances: Vec<Instance>, groups: &[Group]) -> (Vec<Vec<Cache>>, Vec<Cache>) {
    let mut per_group: Vec<Buckets> = groups.iter().map(|_| Buckets::new()).collect();
    let mut shared = Buckets::new();
    for instance in instances {
        let owners: BTreeSet<usize> = instance
            .cpus
            .iter()
            .filter_map(|c| groups.iter().position(|g| g.cpus.contains(c)))
            .collect();
        let bucket = match (owners.len(), owners.first()) {
            (1, Some(&i)) => &mut per_group[i],
            _ => &mut shared,
        };
        bucket
            .entry((instance.level, instance.kind))
            .or_default()
            .push(instance);
    }
    (
        per_group.into_iter().map(summarize).collect(),
        summarize(shared),
    )
}

/// Physical cores among `cpus`: their distinct sibling sets, when every one is known.
fn sharer_cores(cpus: &[u32], topo: &BTreeMap<u32, CpuTopo>) -> Option<usize> {
    let sets: Option<BTreeSet<&Vec<u32>>> = cpus
        .iter()
        .map(|c| topo.get(c)?.siblings.as_ref())
        .collect();
    sets.map(|s| s.len())
}

fn same_shape(a: &Instance, b: &Instance) -> bool {
    a.size.as_ref().map(|f| f.value) == b.size.as_ref().map(|f| f.value)
        && a.cpus.len() == b.cpus.len()
        && a.cores == b.cores
}

/// One entry per distinct shape of cache. Instances of one level can differ (an X3D chiplet's
/// larger L3, Meteor Lake's two-core low-power module, a half-offline L3), so the first instance
/// is never taken to stand for the rest.
fn summarize(buckets: Buckets) -> Vec<Cache> {
    let mut caches = Vec::new();
    for ((level, kind), instances) in buckets {
        let mut shapes: Vec<(&Instance, usize)> = Vec::new();
        for instance in &instances {
            match shapes
                .iter_mut()
                .find(|(shape, _)| same_shape(shape, instance))
            {
                Some((_, n)) => *n += 1,
                None => shapes.push((instance, 1)),
            }
        }
        caches.extend(shapes.into_iter().map(|(first, n)| Cache {
            level,
            kind,
            size: first.size.clone(),
            shared_by: count(first.cpus.len()),
            cores: first.cores.and_then(count),
            instances: count(n),
        }));
    }
    caches
}

/// A cpufreq value (kHz on disk) as hertz. Values outside 100 MHz to 10 GHz are rejected.
fn frequency(r: &mut Reader, cpu: u32, file: &str) -> F<Hertz> {
    let path = format!("{CPU_DIR}/cpu{cpu}/cpufreq/{file}");
    let khz: u64 = r.number(&path)?;
    let hz = khz.saturating_mul(1000);
    if !(100_000_000..=10_000_000_000).contains(&hz) {
        r.reject(&path, "implausible frequency");
        return None;
    }
    Some(Fact::detected(Hertz(hz), path))
}

/// Base and max clocks. Current frequency is deliberately not collected: it changes every
/// moment, so it is monitoring rather than a fact, and a dump could never replay it.
fn clocks(r: &mut Reader, cpus: &[u32]) -> Clocks {
    let Some(&first) = cpus.first() else {
        return Clocks::default();
    };
    let max = cpus
        .iter()
        .filter_map(|&c| frequency(r, c, "cpuinfo_max_freq"))
        .max_by_key(|f| f.value);
    Clocks {
        base: frequency(r, first, "base_frequency"),
        max,
        current: None,
    }
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;

    use super::*;
    use crate::test_support::fixture;

    fn value<T: Clone>(fact: &F<T>) -> Option<T> {
        fact.as_ref().map(|f| f.value.clone())
    }

    fn files(pairs: &[(&str, &str)]) -> BTreeMap<String, String> {
        pairs
            .iter()
            .map(|(k, v)| (k.to_string(), v.to_string()))
            .collect()
    }

    /// A 4-CPU ARM machine; `implementer`/`part` go into every cpuinfo block.
    fn arm(implementer: &str, part: &str, extra: &[(&str, &str)]) -> BTreeMap<String, String> {
        let block = |n: u32| {
            format!(
                "processor\t: {n}\nFeatures\t: fp asimd aes\nCPU implementer\t: {implementer}\nCPU architecture: 8\nCPU variant\t: 0x0\nCPU part\t: {part}\nCPU revision\t: 0\n\n"
            )
        };
        let mut map = files(&[("/sys/devices/system/cpu/online", "0-3")]);
        map.insert("/proc/cpuinfo".into(), (0..4).map(block).collect());
        for cpu in 0..4 {
            map.insert(
                format!("/sys/devices/system/cpu/cpu{cpu}/topology/thread_siblings_list"),
                cpu.to_string(),
            );
            map.insert(
                format!("/sys/devices/system/cpu/cpu{cpu}/topology/physical_package_id"),
                "0".into(),
            );
        }
        map.extend(files(extra));
        map
    }

    #[test]
    fn intel_identity() {
        let cpu = fixture("linux-x86-intel-hybrid");
        let id = &cpu.identity;
        assert_eq!(
            value(&id.name).as_deref(),
            Some("12th Gen Intel(R) Core(TM) i7-12700K")
        );
        assert_eq!(
            id.vendor,
            Some(Fact::detected(
                "Intel".to_string(),
                "/proc/cpuinfo:vendor_id"
            ))
        );
        assert_eq!(
            (
                value(&id.x86_family),
                value(&id.x86_model),
                value(&id.x86_stepping)
            ),
            (Some(6), Some(151), Some(2))
        );
        assert_eq!(value(&id.microcode).as_deref(), Some("0x2f"));
        assert_eq!(
            id.arch,
            Some(Fact::detected(
                "x86_64".to_string(),
                "/proc/sys/kernel/arch"
            ))
        );
        assert_eq!(id.hypervisor, None);
    }

    #[test]
    fn intel_topology_counts_cores_by_sibling_sets() {
        let t = fixture("linux-x86-intel-hybrid").topology;
        assert_eq!(
            (
                value(&t.sockets),
                value(&t.physical_cores),
                value(&t.logical_cpus)
            ),
            (Some(1), Some(12), Some(20))
        );
        assert_eq!(
            t.smt_per_core, None,
            "20 threads on 12 cores is not a whole SMT factor"
        );
    }

    #[test]
    fn gce_hypervisor_numa_and_non_adjacent_siblings() {
        let cpu = fixture("linux-x86-amd-gce");
        assert_eq!(
            cpu.identity.hypervisor,
            Some(Fact::detected(
                "Google Compute Engine".to_string(),
                "/sys/class/dmi/id/product_name"
            ))
        );
        let t = &cpu.topology;
        assert_eq!(
            (
                value(&t.physical_cores),
                value(&t.logical_cpus),
                value(&t.smt_per_core)
            ),
            (Some(8), Some(16), Some(2))
        );
        let nodes: Vec<(u32, Vec<u32>)> = t
            .numa_nodes
            .iter()
            .map(|n| (n.id, n.cpus.clone()))
            .collect();
        assert_eq!(
            nodes,
            [
                (0, vec![0, 1, 2, 3, 8, 9, 10, 11]),
                (1, vec![4, 5, 6, 7, 12, 13, 14, 15])
            ]
        );
    }

    #[test]
    fn x86_features_come_from_flags() {
        let cpu = fixture("linux-x86-intel-hybrid");
        assert!(cpu.features.raw.iter().any(|f| f == "avx2"));
        let crypto: Vec<&str> = cpu
            .features
            .groups
            .iter()
            .find(|g| g.group == crate::model::FeatureGroup::Crypto)
            .unwrap()
            .features
            .iter()
            .map(|f| f.name.as_str())
            .collect();
        assert_eq!(
            crypto,
            [
                "AES", "PCLMUL", "VAES", "VPCLMUL", "SHA-NI", "RDRAND", "RDSEED"
            ]
        );
    }

    #[test]
    fn arm_cores_are_named_from_the_midr_table() {
        let cpu = collect(&arm("0x41", "0xd0c", &[]));
        let db = Origin::Database("arm-midr@2026-09");
        assert_eq!(
            cpu.identity.name,
            Some(Fact {
                value: "Neoverse-N1".to_string(),
                origin: db.clone()
            })
        );
        assert_eq!(
            cpu.identity.vendor,
            Some(Fact {
                value: "ARM".to_string(),
                origin: db
            })
        );
        assert_eq!(
            cpu.identity.arch,
            Some(Fact::derived("aarch64".to_string())),
            "no /proc/sys/kernel/arch: derived"
        );
    }

    #[test]
    fn apple_vm_part_zero_is_not_named() {
        let cpu = collect(&arm(
            "0x61",
            "0x000",
            &[
                ("/sys/class/dmi/id/sys_vendor", "Apple Inc."),
                (
                    "/sys/class/dmi/id/product_name",
                    "Apple Virtualization Generic Platform",
                ),
            ],
        ));
        assert_eq!(cpu.identity.name, None);
        assert_eq!(value(&cpu.identity.vendor).as_deref(), Some("Apple"));
        assert_eq!(
            value(&cpu.identity.hypervisor).as_deref(),
            Some("Apple Virtualization Generic Platform")
        );
        assert!(cpu.is_identified(), "identified by its CPU count");
    }

    #[test]
    fn x86_trusts_the_hypervisor_flag_not_dmi() {
        let mut map = files(&[
            (
                "/proc/cpuinfo",
                "processor\t: 0\nvendor_id\t: GenuineIntel\nmodel name\t: Metal\nflags\t: fpu lm\n\n",
            ),
            ("/sys/devices/system/cpu/online", "0"),
            ("/sys/class/dmi/id/product_name", "Google Compute Engine"),
        ]);
        assert_eq!(
            collect(&map).identity.hypervisor,
            None,
            "bare-metal cloud hosts keep cloud DMI names"
        );
        map.insert(
            "/proc/cpuinfo".into(),
            "processor\t: 0\nvendor_id\t: GenuineIntel\nflags\t: fpu lm hypervisor\n\n".into(),
        );
        assert_eq!(
            value(&collect(&map).identity.hypervisor).as_deref(),
            Some("Google Compute Engine")
        );
    }

    #[test]
    fn garbage_values_are_rejected_with_diagnostics() {
        let map = files(&[
            (
                "/proc/cpuinfo",
                "processor\t: 0\nvendor_id\t: GenuineIntel\nmodel name\t: Evil\u{1b}]0;x\u{7}\ncpu family\t: six\nflags\t: lm\n\n",
            ),
            ("/sys/devices/system/cpu/online", "0-4294967295"),
            (
                "/sys/devices/system/cpu/cpu0/topology/thread_siblings_list",
                "0",
            ),
            (
                "/sys/devices/system/cpu/cpu1/topology/thread_siblings_list",
                "1",
            ),
        ]);
        let cpu = collect(&map);
        assert_eq!(cpu.identity.name, None);
        assert_eq!(cpu.identity.x86_family, None);
        assert_eq!(
            value(&cpu.topology.logical_cpus),
            Some(2),
            "falls back to counting cpuN directories"
        );
        let from: Vec<&str> = cpu.diagnostics.iter().map(|d| d.from.as_str()).collect();
        assert!(from.contains(&"/sys/devices/system/cpu/online"), "{from:?}");
        assert!(from.contains(&"/proc/cpuinfo:model name"), "{from:?}");
        assert!(from.contains(&"/proc/cpuinfo:cpu family"), "{from:?}");
    }

    #[test]
    fn missing_cpuinfo_still_identifies_by_cpu_count() {
        let cpu = collect(&files(&[("/sys/devices/system/cpu/online", "0-7")]));
        assert_eq!(value(&cpu.topology.logical_cpus), Some(8));
        assert!(cpu.is_identified());
    }

    use crate::model::{CacheKind, CoreKind};
    use crate::units::{Bytes, Hertz};

    fn cache(
        cluster: &crate::model::Cluster,
        level: u8,
        kind: CacheKind,
    ) -> (Option<Bytes>, Option<u32>, Option<u32>) {
        let k = cluster
            .caches
            .iter()
            .find(|k| k.level == level && k.kind == kind)
            .expect("cache present");
        (value(&k.size), value(&k.shared_by), value(&k.instances))
    }

    #[test]
    fn intel_hybrid_has_two_core_types_and_a_shared_l3() {
        let cpu = fixture("linux-x86-intel-hybrid");
        let c = &cpu.clusters;
        assert_eq!(c.len(), 2);
        assert_eq!(
            (c[0].kind, value(&c[0].cores), value(&c[0].threads)),
            (CoreKind::Performance, Some(8), Some(16))
        );
        assert_eq!(
            (c[1].kind, value(&c[1].cores), value(&c[1].threads)),
            (CoreKind::Efficiency, Some(4), Some(4))
        );
        assert_eq!(
            cache(&c[0], 2, CacheKind::Unified),
            (Some(Bytes(1280 << 10)), Some(2), Some(8))
        );
        assert_eq!(
            cache(&c[1], 2, CacheKind::Unified),
            (Some(Bytes(2 << 20)), Some(4), Some(1))
        );
        assert!(
            c.iter().all(|cl| cl.caches.iter().all(|k| k.level != 3)),
            "L3 spans both types"
        );
        let l3 = &cpu.shared_caches;
        assert_eq!(l3.len(), 1);
        assert_eq!(
            (
                l3[0].level,
                value(&l3[0].size),
                value(&l3[0].shared_by),
                value(&l3[0].instances)
            ),
            (3, Some(Bytes(25 << 20)), Some(20), Some(1))
        );
    }

    #[test]
    fn intel_clocks_per_core_type() {
        let c = fixture("linux-x86-intel-hybrid").clusters;
        assert_eq!(
            (value(&c[0].clock.base), value(&c[0].clock.max)),
            (Some(Hertz(3_600_000_000)), Some(Hertz(5_000_000_000)))
        );
        assert_eq!(
            (value(&c[1].clock.base), value(&c[1].clock.max)),
            (Some(Hertz(2_700_000_000)), Some(Hertz(3_800_000_000)))
        );
        assert_eq!(
            c[0].clock.current, None,
            "current frequency is not a static fact"
        );
    }

    #[test]
    fn gce_is_uniform_with_one_l3_per_node() {
        let cpu = fixture("linux-x86-amd-gce");
        assert_eq!(cpu.clusters.len(), 1);
        assert_eq!(cpu.clusters[0].kind, CoreKind::Uniform);
        assert_eq!(
            cache(&cpu.clusters[0], 3, CacheKind::Unified),
            (Some(Bytes(16 << 20)), Some(8), Some(2))
        );
        assert!(cpu.shared_caches.is_empty());
        assert!(
            cpu.clusters[0].clock.is_empty(),
            "the VM exposes no cpufreq"
        );
    }

    #[test]
    fn arm_big_little_splits_by_capacity_and_part() {
        let block = |n: u32, part: &str| {
            format!(
                "processor\t: {n}\nFeatures\t: fp asimd\nCPU implementer\t: 0x41\nCPU architecture: 8\nCPU part\t: {part}\n\n"
            )
        };
        let mut map = files(&[("/sys/devices/system/cpu/online", "0-7")]);
        let cpuinfo: String = (0..8)
            .map(|n| block(n, if n < 4 { "0xd0b" } else { "0xd05" }))
            .collect();
        map.insert("/proc/cpuinfo".into(), cpuinfo);
        for cpu in 0..8u32 {
            map.insert(
                format!("/sys/devices/system/cpu/cpu{cpu}/cpu_capacity"),
                if cpu < 4 { "1024" } else { "446" }.into(),
            );
            map.insert(
                format!("/sys/devices/system/cpu/cpu{cpu}/topology/thread_siblings_list"),
                cpu.to_string(),
            );
        }
        let c = collect(&map).clusters;
        let shown: Vec<(CoreKind, Option<String>, Option<u32>)> = c
            .iter()
            .map(|cl| (cl.kind, value(&cl.name), value(&cl.cores)))
            .collect();
        assert_eq!(
            shown,
            [
                (
                    CoreKind::Performance,
                    Some("Cortex-A76".to_string()),
                    Some(4)
                ),
                (
                    CoreKind::Efficiency,
                    Some("Cortex-A55".to_string()),
                    Some(4)
                ),
            ]
        );
    }

    #[test]
    fn caches_without_a_size_keep_their_sharing() {
        let mut map = arm("0x61", "0x000", &[]);
        for cpu in 0..4 {
            map.insert(
                format!("/sys/devices/system/cpu/cpu{cpu}/cache/index0/level"),
                "2".into(),
            );
            map.insert(
                format!("/sys/devices/system/cpu/cpu{cpu}/cache/index0/type"),
                "Unified".into(),
            );
            map.insert(
                format!("/sys/devices/system/cpu/cpu{cpu}/cache/index0/shared_cpu_list"),
                "0-3".into(),
            );
        }
        let cpu = collect(&map);
        assert_eq!(
            cache(&cpu.clusters[0], 2, CacheKind::Unified),
            (None, Some(4), Some(1))
        );
        assert!(
            cpu.diagnostics.is_empty(),
            "a missing size is not an error: {:?}",
            cpu.diagnostics
        );
    }

    #[test]
    fn offline_cpus_do_not_count_as_sharers() {
        let mut map = arm(
            "0x41",
            "0xd0c",
            &[("/sys/devices/system/cpu/online", "0-2")],
        );
        for cpu in 0..3 {
            map.insert(
                format!("/sys/devices/system/cpu/cpu{cpu}/cache/index0/level"),
                "2".into(),
            );
            map.insert(
                format!("/sys/devices/system/cpu/cpu{cpu}/cache/index0/type"),
                "Unified".into(),
            );
            map.insert(
                format!("/sys/devices/system/cpu/cpu{cpu}/cache/index0/size"),
                "1024K".into(),
            );
            map.insert(
                format!("/sys/devices/system/cpu/cpu{cpu}/cache/index0/shared_cpu_list"),
                "0-3".into(),
            );
        }
        let cpu = collect(&map);
        assert_eq!(value(&cpu.clusters[0].threads), Some(3));
        assert_eq!(
            cache(&cpu.clusters[0], 2, CacheKind::Unified),
            (Some(Bytes(1 << 20)), Some(3), Some(1))
        );
    }

    #[test]
    fn implausible_frequencies_are_rejected() {
        let map = arm(
            "0x41",
            "0xd0c",
            &[
                ("/sys/devices/system/cpu/cpu0/cpufreq/cpuinfo_max_freq", "1"),
                (
                    "/sys/devices/system/cpu/cpu0/cpufreq/base_frequency",
                    "999999999999",
                ),
            ],
        );
        let cpu = collect(&map);
        assert!(cpu.clusters[0].clock.is_empty());
        let from: Vec<&str> = cpu.diagnostics.iter().map(|d| d.from.as_str()).collect();
        assert!(
            from.contains(&"/sys/devices/system/cpu/cpu0/cpufreq/cpuinfo_max_freq"),
            "{from:?}"
        );
        assert!(
            from.contains(&"/sys/devices/system/cpu/cpu0/cpufreq/base_frequency"),
            "{from:?}"
        );
    }

    /// The 4-CPU ARM machine with one level-3 cache per `(cpu list, size)`.
    fn with_l3(instances: &[(&str, &str)]) -> BTreeMap<String, String> {
        let mut map = arm("0x41", "0xd0c", &[]);
        for (cpus, size) in instances {
            for cpu in crate::collect::sysfs::parse_cpu_list(cpus).unwrap() {
                let base = format!("/sys/devices/system/cpu/cpu{cpu}/cache/index0");
                map.insert(format!("{base}/level"), "3".into());
                map.insert(format!("{base}/type"), "Unified".into());
                map.insert(format!("{base}/size"), (*size).into());
                map.insert(format!("{base}/shared_cpu_list"), (*cpus).into());
            }
        }
        map
    }

    fn l3_shapes(cpu: &Cpu) -> Vec<(Option<Bytes>, Option<u32>, Option<u32>)> {
        cpu.clusters[0]
            .caches
            .iter()
            .filter(|k| k.level == 3)
            .map(|k| (value(&k.size), value(&k.shared_by), value(&k.instances)))
            .collect()
    }

    #[test]
    fn caches_of_different_sizes_are_listed_separately() {
        let cpu = collect(&with_l3(&[("0-1", "98304K"), ("2-3", "32768K")]));
        assert_eq!(
            l3_shapes(&cpu),
            [
                (Some(Bytes(96 << 20)), Some(2), Some(1)),
                (Some(Bytes(32 << 20)), Some(2), Some(1))
            ]
        );
    }

    #[test]
    fn caches_shared_by_different_numbers_of_cpus_are_listed_separately() {
        let cpu = collect(&with_l3(&[
            ("0-1", "2048K"),
            ("2", "2048K"),
            ("3", "2048K"),
        ]));
        assert_eq!(
            l3_shapes(&cpu),
            [
                (Some(Bytes(2 << 20)), Some(2), Some(1)),
                (Some(Bytes(2 << 20)), Some(1), Some(2))
            ]
        );
    }

    #[test]
    fn caches_count_the_cores_that_share_them() {
        // 2 cores x 2 threads; siblings only in core_cpus_list (older or unusual kernels).
        let mut map = files(&[("/sys/devices/system/cpu/online", "0-3")]);
        for cpu in 0..4u32 {
            let pair = if cpu < 2 { "0-1" } else { "2-3" };
            let base = format!("/sys/devices/system/cpu/cpu{cpu}");
            map.insert(format!("{base}/topology/core_cpus_list"), pair.into());
            for (index, level, kind, size, shared) in [
                (0, "1", "Data", "32K", pair),
                (1, "3", "Unified", "16384K", "0-3"),
            ] {
                let cache = format!("{base}/cache/index{index}");
                map.insert(format!("{cache}/level"), level.into());
                map.insert(format!("{cache}/type"), kind.into());
                map.insert(format!("{cache}/size"), size.into());
                map.insert(format!("{cache}/shared_cpu_list"), shared.into());
            }
        }
        let cpu = collect(&map);
        assert_eq!(value(&cpu.topology.physical_cores), Some(2));
        let cores: Vec<(u8, Option<u32>, Option<u32>)> = cpu.clusters[0]
            .caches
            .iter()
            .map(|k| (k.level, value(&k.shared_by), value(&k.cores)))
            .collect();
        assert_eq!(cores, [(1, Some(2), Some(1)), (3, Some(4), Some(2))]);
    }
}
