//! Linux collector: `/proc/cpuinfo`, `/sys/devices/system/{cpu,node}` and two DMI strings.
//! Compiled on every OS, so Linux fixtures are tested on any machine.

use std::collections::{BTreeMap, BTreeSet};

use super::sysfs::{parse_cpu_list, parse_cpuinfo};
use super::{count, features, ratio};
use crate::db;
use crate::model::{Cpu, Diagnostic, F, Fact, Identity, NumaNode, Origin, Topology};
use crate::source::Fs;

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
    let features = features::group(flags(&info));
    Cpu {
        identity,
        topology,
        clusters: Vec::new(),
        shared_caches: Vec::new(),
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
                siblings: r.cpu_list(&format!("{base}/thread_siblings_list")),
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
}
