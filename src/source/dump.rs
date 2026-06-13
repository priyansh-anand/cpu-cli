//! `cpu --dump`: capture this machine's raw CPU data as a snapshot.
//!
//! Capture is an allowlist that deliberately covers more than today's collector reads, so old
//! snapshots keep working as collectors learn to read more. It is also the privacy guarantee:
//! nothing outside it (hostname, serial numbers, UUIDs, kernel addresses) is ever written. macOS
//! captures whole sysctl trees; Linux lists files one by one, because sysfs directories also hold
//! kernel addresses.

use std::collections::BTreeMap;
use std::io;

use super::snapshot::{Meta, Snapshot};
use super::{Fs, SysctlValue};

/// sysctl trees captured whole.
pub const SYSCTL_TREES: &[&str] = &["hw", "machdep.cpu"];
/// Individual sysctl keys captured.
pub const SYSCTL_KEYS: &[&str] = &["sysctl.proc_translated", "kern.osrelease"];

pub fn allowed_sysctl(key: &str) -> bool {
    SYSCTL_KEYS.contains(&key)
        || SYSCTL_TREES.iter().any(|tree| {
            key.strip_prefix(tree)
                .is_some_and(|rest| rest.starts_with('.'))
        })
}

/// IOKit properties captured: the power manager's frequency tables. Nothing else in the registry
/// (serial numbers, UUIDs) is ever read.
pub fn allowed_ioreg(key: &str) -> bool {
    key.strip_prefix("pmgr:")
        .is_some_and(|k| k.starts_with("voltage-states"))
}

const CPUINFO: &str = "/proc/cpuinfo";
const OSRELEASE: &str = "/proc/sys/kernel/osrelease";
const CPU_DIR: &str = "/sys/devices/system/cpu";
const NODE_DIR: &str = "/sys/devices/system/node";

/// Individual Linux files captured.
pub const LINUX_FILES: &[&str] = &[
    CPUINFO,
    OSRELEASE,
    "/proc/sys/kernel/arch",
    "/sys/devices/system/cpu/online",
    "/sys/devices/system/cpu/possible",
    "/sys/devices/cpu_core/cpus",
    "/sys/devices/cpu_atom/cpus",
    "/sys/devices/cpu_lowpower/cpus",
    "/sys/class/dmi/id/sys_vendor",
    "/sys/class/dmi/id/product_name",
];
/// Files captured for every `cpuN`. Listed one by one: sysfs also holds kernel addresses
/// (`crash_notes`), so directories are never swept.
const CPU_FILES: &[&str] = &[
    "topology/physical_package_id",
    "topology/core_id",
    "topology/die_id",
    "topology/cluster_id",
    "topology/thread_siblings_list",
    "topology/core_cpus_list",
    "cpu_capacity",
    "regs/identification/midr_el1",
    "cpufreq/cpuinfo_max_freq",
    "cpufreq/cpuinfo_min_freq",
    "cpufreq/base_frequency",
];
/// Files captured for every `cpuN/cache/indexM`.
const CACHE_FILES: &[&str] = &[
    "level",
    "type",
    "size",
    "shared_cpu_list",
    "ways_of_associativity",
    "coherency_line_size",
];

/// `name` is `prefix` followed by a decimal number, e.g. `cpu12`.
fn numbered(name: &str, prefix: &str) -> bool {
    name.strip_prefix(prefix)
        .is_some_and(|n| !n.is_empty() && n.bytes().all(|b| b.is_ascii_digit()))
}

/// Whether `--dump` may capture this Linux file.
pub fn allowed_file(path: &str) -> bool {
    if LINUX_FILES.contains(&path) {
        return true;
    }
    if let Some(rest) = path.strip_prefix(&format!("{CPU_DIR}/")) {
        let Some((cpu, file)) = rest.split_once('/') else {
            return false;
        };
        if !numbered(cpu, "cpu") {
            return false;
        }
        if CPU_FILES.contains(&file) {
            return true;
        }
        return file
            .strip_prefix("cache/")
            .and_then(|f| f.split_once('/'))
            .is_some_and(|(index, name)| numbered(index, "index") && CACHE_FILES.contains(&name));
    }
    path.strip_prefix(&format!("{NODE_DIR}/"))
        .and_then(|rest| rest.split_once('/'))
        .is_some_and(|(node, file)| numbered(node, "node") && file == "cpulist")
}

/// Every allowlisted Linux file readable through `fs`, with `/proc/cpuinfo` scrubbed.
pub fn capture_files(fs: &dyn Fs) -> BTreeMap<String, String> {
    let mut paths: Vec<String> = LINUX_FILES.iter().map(|p| p.to_string()).collect();
    for cpu in fs.list(CPU_DIR).into_iter().filter(|n| numbered(n, "cpu")) {
        let base = format!("{CPU_DIR}/{cpu}");
        paths.extend(CPU_FILES.iter().map(|f| format!("{base}/{f}")));
        for index in fs
            .list(&format!("{base}/cache"))
            .into_iter()
            .filter(|n| numbered(n, "index"))
        {
            paths.extend(
                CACHE_FILES
                    .iter()
                    .map(|f| format!("{base}/cache/{index}/{f}")),
            );
        }
    }
    for node in fs
        .list(NODE_DIR)
        .into_iter()
        .filter(|n| numbered(n, "node"))
    {
        paths.push(format!("{NODE_DIR}/{node}/cpulist"));
    }
    paths
        .into_iter()
        .filter(|p| allowed_file(p))
        .filter_map(|p| {
            let text = fs.read(&p)?;
            let text = if p == CPUINFO {
                scrub_cpuinfo(&text)
            } else {
                text
            };
            Some((p, text))
        })
        .collect()
}

/// Drops `Serial` lines (a Raspberry Pi's board serial number) from `/proc/cpuinfo`.
fn scrub_cpuinfo(text: &str) -> String {
    text.lines()
        .filter(|line| line.split(':').next().map(str::trim) != Some("Serial"))
        .map(|line| format!("{line}\n"))
        .collect()
}

pub fn capture_live() -> io::Result<Snapshot> {
    let sysctl = capture_sysctl()?;
    let files = capture_live_files();
    let ioreg = capture_ioreg();
    let kernel = match sysctl.get("kern.osrelease") {
        Some(SysctlValue::Str(s)) => Some(s.clone()),
        _ => files.get(OSRELEASE).map(|s| s.trim().to_string()),
    };
    Ok(Snapshot {
        meta: Meta::current(kernel),
        sysctl,
        files,
        ioreg,
    })
}

#[cfg(target_os = "macos")]
fn capture_ioreg() -> BTreeMap<String, Vec<u8>> {
    super::ioreg::LiveIoReg::new()
        .properties("pmgr")
        .into_iter()
        .map(|(key, bytes)| (format!("pmgr:{key}"), bytes))
        .filter(|(key, _)| allowed_ioreg(key))
        .collect()
}

#[cfg(not(target_os = "macos"))]
fn capture_ioreg() -> BTreeMap<String, Vec<u8>> {
    BTreeMap::new()
}

#[cfg(target_os = "linux")]
fn capture_live_files() -> BTreeMap<String, String> {
    capture_files(&super::files::LiveFs::root())
}

#[cfg(not(target_os = "linux"))]
fn capture_live_files() -> BTreeMap<String, String> {
    BTreeMap::new()
}

#[cfg(target_os = "macos")]
fn capture_sysctl() -> io::Result<BTreeMap<String, SysctlValue>> {
    use super::Sysctl;
    use super::macos::{LiveSysctl, list_names};

    let names = list_names(SYSCTL_TREES)?;
    Ok(names
        .iter()
        .map(String::as_str)
        .chain(SYSCTL_KEYS.iter().copied())
        .filter(|key| allowed_sysctl(key))
        .filter_map(|key| LiveSysctl.get(key).map(|value| (key.to_string(), value)))
        .collect())
}

#[cfg(not(target_os = "macos"))]
fn capture_sysctl() -> io::Result<BTreeMap<String, SysctlValue>> {
    Ok(BTreeMap::new())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn allowlist_admits_cpu_keys_only() {
        assert!(allowed_sysctl("hw.ncpu"));
        assert!(allowed_sysctl("hw.perflevel0.name"));
        assert!(allowed_sysctl("machdep.cpu.brand_string"));
        assert!(allowed_sysctl("kern.osrelease"));
        assert!(!allowed_sysctl("kern.hostname"));
        assert!(!allowed_sysctl("kern.uuid"));
        assert!(!allowed_sysctl("hwx.anything"));
        assert!(!allowed_sysctl("machdep.cpux"));
    }

    #[test]
    fn capture_describes_this_machine() {
        let snap = capture_live().unwrap();
        assert_eq!(snap.meta.os, std::env::consts::OS);
        assert!(
            snap.sysctl.keys().all(|k| allowed_sysctl(k)),
            "{:?}",
            snap.sysctl.keys()
        );
        if cfg!(target_os = "macos") {
            assert!(snap.sysctl.contains_key("machdep.cpu.brand_string"));
        }
    }

    use std::path::Path;

    use crate::source::files::LiveFs;
    use crate::source::{Fs, Os, Sources, Stub};

    #[test]
    fn linux_allowlist() {
        assert!(allowed_file("/proc/cpuinfo"));
        assert!(allowed_file("/sys/devices/system/cpu/online"));
        assert!(allowed_file(
            "/sys/devices/system/cpu/cpu12/topology/thread_siblings_list"
        ));
        assert!(allowed_file(
            "/sys/devices/system/cpu/cpu0/cache/index3/shared_cpu_list"
        ));
        assert!(allowed_file("/sys/devices/system/node/node1/cpulist"));
        assert!(!allowed_file("/sys/devices/system/cpu/cpu0/crash_notes"));
        assert!(!allowed_file("/sys/devices/system/cpu/cpux/online"));
        assert!(!allowed_file(
            "/sys/devices/system/cpu/cpu0/cache/index0/uevent"
        ));
        assert!(!allowed_file("/sys/devices/system/node/node0/meminfo"));
        assert!(!allowed_file("/etc/hostname"));
    }

    #[test]
    fn capture_skips_unlisted_files_and_serials() {
        let dir = tempfile::tempdir().unwrap();
        let put = |path: &str, text: &str| {
            let target = dir.path().join(path.trim_start_matches('/'));
            std::fs::create_dir_all(target.parent().unwrap()).unwrap();
            std::fs::write(target, text).unwrap();
        };
        put(
            "/proc/cpuinfo",
            "processor\t: 0\nHardware\t: BCM2835\nSerial\t\t: 10000000abcdef01\nModel\t\t: Raspberry Pi 4\n",
        );
        put("/sys/devices/system/cpu/online", "0\n");
        put(
            "/sys/devices/system/cpu/cpu0/topology/thread_siblings_list",
            "0\n",
        );
        put(
            "/sys/devices/system/cpu/cpu0/crash_notes",
            "ffff8880bfa1b000\n",
        );
        put("/sys/devices/system/cpu/cpu0/cache/index0/size", "32K\n");
        put("/sys/devices/system/cpu/cpu0/cache/index0/uevent", "x\n");

        let files = capture_files(&LiveFs::at(dir.path()));
        let paths: Vec<&str> = files.keys().map(String::as_str).collect();
        assert_eq!(
            paths,
            [
                "/proc/cpuinfo",
                "/sys/devices/system/cpu/cpu0/cache/index0/size",
                "/sys/devices/system/cpu/cpu0/topology/thread_siblings_list",
                "/sys/devices/system/cpu/online",
            ]
        );
        assert!(
            !files["/proc/cpuinfo"].contains("Serial")
                && files["/proc/cpuinfo"].contains("Raspberry Pi 4")
        );
    }

    #[test]
    fn the_linux_collector_reads_only_captured_files() {
        use std::cell::RefCell;
        use std::collections::BTreeMap;
        use std::rc::Rc;

        struct Recording {
            files: BTreeMap<String, String>,
            reads: Rc<RefCell<Vec<String>>>,
        }
        impl Fs for Recording {
            fn read(&self, path: &str) -> Option<String> {
                self.reads.borrow_mut().push(path.to_string());
                self.files.read(path)
            }
            fn list(&self, dir: &str) -> Vec<String> {
                self.files.list(dir)
            }
        }

        for name in [
            "linux-x86-intel-hybrid",
            "linux-x86-amd-gce",
            "linux-x86-gcp-epyc-7b12",
            "linux-arm64-oci-a1",
            "linux-arm64-apple-vm",
        ] {
            let path = Path::new(env!("CARGO_MANIFEST_DIR"))
                .join("tests/fixtures")
                .join(name);
            let snapshot = Snapshot::open(&path).unwrap();
            let reads = Rc::new(RefCell::new(Vec::new()));
            let fs = Recording {
                files: snapshot.files,
                reads: Rc::clone(&reads),
            };
            let sources = Sources {
                os: Os::Linux,
                fs: Box::new(fs),
                sysctl: Box::new(Stub),
                cpuid: Box::new(Stub),
                ioreg: Box::new(Stub),
            };
            crate::collect::collect(&sources);
            let missed: Vec<String> = reads
                .borrow()
                .iter()
                .filter(|p| !allowed_file(p))
                .cloned()
                .collect();
            assert!(
                missed.is_empty(),
                "{name}: the collector reads files --dump never captures: {missed:?}"
            );
        }
    }

    #[test]
    fn ioreg_allowlist_is_frequency_tables_only() {
        assert!(allowed_ioreg("pmgr:voltage-states5-sram"));
        assert!(allowed_ioreg("pmgr:voltage-states11"));
        assert!(!allowed_ioreg("pmgr:compatible"));
        assert!(!allowed_ioreg(
            "IOPlatformExpertDevice:IOPlatformSerialNumber"
        ));
    }
}
