//! Sources are the only code that touches the live system. Each exists in a live form and a
//! recorded form (a snapshot), so collectors run identically on this machine and on fixtures.

use std::collections::BTreeMap;
use std::path::Path;

pub mod snapshot;

pub use snapshot::{Snapshot, SnapshotError};

/// A sysctl value. Arrays (e.g. `hw.cachesize`) are stored as space-separated strings.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SysctlValue {
    Int(i64),
    Str(String),
}

impl SysctlValue {
    pub fn as_i64(&self) -> Option<i64> {
        match self {
            SysctlValue::Int(n) => Some(*n),
            SysctlValue::Str(s) => s.trim().parse().ok(),
        }
    }
}

pub trait Sysctl {
    fn get(&self, key: &str) -> Option<SysctlValue>;

    /// Every key starting with `prefix`, e.g. `hw.optional.arm.`.
    fn keys_with_prefix(&self, prefix: &str) -> Vec<String>;

    fn int(&self, key: &str) -> Option<i64> {
        self.get(key)?.as_i64()
    }

    fn string(&self, key: &str) -> Option<String> {
        match self.get(key)? {
            SysctlValue::Str(s) => Some(s),
            SysctlValue::Int(n) => Some(n.to_string()),
        }
    }
}

pub trait Fs {
    fn read(&self, path: &str) -> Option<String>;
    /// Names of the direct children of `dir`.
    fn list(&self, dir: &str) -> Vec<String>;
}

pub trait Cpuid {
    fn query(&self, leaf: u32, subleaf: u32) -> Option<[u32; 4]>;
}

pub trait IoReg {
    fn property(&self, service: &str, key: &str) -> Option<Vec<u8>>;
}

/// Stands in for a source that doesn't exist on this OS. Knows nothing.
pub struct Stub;

impl Sysctl for Stub {
    fn get(&self, _: &str) -> Option<SysctlValue> {
        None
    }
    fn keys_with_prefix(&self, _: &str) -> Vec<String> {
        Vec::new()
    }
}

impl Fs for Stub {
    fn read(&self, _: &str) -> Option<String> {
        None
    }
    fn list(&self, _: &str) -> Vec<String> {
        Vec::new()
    }
}

impl Cpuid for Stub {
    fn query(&self, _: u32, _: u32) -> Option<[u32; 4]> {
        None
    }
}

impl IoReg for Stub {
    fn property(&self, _: &str, _: &str) -> Option<Vec<u8>> {
        None
    }
}

/// Recorded sysctl: a snapshot's `sysctl.toml`, or a test's literal map.
impl Sysctl for BTreeMap<String, SysctlValue> {
    fn get(&self, key: &str) -> Option<SysctlValue> {
        BTreeMap::get(self, key).cloned()
    }
    fn keys_with_prefix(&self, prefix: &str) -> Vec<String> {
        self.keys()
            .filter(|k| k.starts_with(prefix))
            .cloned()
            .collect()
    }
}

/// Recorded files keyed by absolute path, e.g. `/proc/cpuinfo`.
impl Fs for BTreeMap<String, String> {
    fn read(&self, path: &str) -> Option<String> {
        BTreeMap::get(self, path).cloned()
    }
    fn list(&self, dir: &str) -> Vec<String> {
        let prefix = format!("{}/", dir.trim_end_matches('/'));
        let mut names: Vec<String> = self
            .keys()
            .filter_map(|k| k.strip_prefix(&prefix))
            .map(|rest| rest.split('/').next().unwrap_or(rest).to_string())
            .collect();
        // Keys are sorted, so every path under one child is adjacent.
        names.dedup();
        names
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Os {
    MacOs,
    Linux,
    Other,
}

impl Os {
    pub fn current() -> Os {
        Os::from_name(std::env::consts::OS)
    }

    /// From `std::env::consts::OS` spelling, as stored in snapshot metadata.
    pub fn from_name(name: &str) -> Os {
        match name {
            "macos" => Os::MacOs,
            "linux" => Os::Linux,
            _ => Os::Other,
        }
    }
}

pub struct Sources {
    pub os: Os,
    pub fs: Box<dyn Fs>,
    pub sysctl: Box<dyn Sysctl>,
    pub cpuid: Box<dyn Cpuid>,
    pub ioreg: Box<dyn IoReg>,
}

impl Sources {
    /// This machine.
    pub fn live() -> Sources {
        Sources {
            os: Os::current(),
            fs: Box::new(Stub),
            sysctl: live_sysctl(),
            cpuid: Box::new(Stub),
            ioreg: Box::new(Stub),
        }
    }

    /// A snapshot directory or `.tar.gz`.
    pub fn recorded(path: &Path) -> Result<Sources, SnapshotError> {
        Snapshot::open(path).map(Snapshot::into_sources)
    }
}

fn live_sysctl() -> Box<dyn Sysctl> {
    Box::new(Stub)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn map_sysctl_reads_ints_strings_and_prefixes() {
        let map = BTreeMap::from([
            ("hw.ncpu".to_string(), SysctlValue::Int(10)),
            ("hw.optional.arm.FEAT_AES".to_string(), SysctlValue::Int(1)),
            ("hw.optional.arm.FEAT_SHA3".to_string(), SysctlValue::Int(1)),
            (
                "machdep.cpu.brand_string".to_string(),
                SysctlValue::Str("Apple M5".into()),
            ),
        ]);
        assert_eq!(map.int("hw.ncpu"), Some(10));
        assert_eq!(map.string("hw.ncpu").as_deref(), Some("10"));
        assert_eq!(
            map.string("machdep.cpu.brand_string").as_deref(),
            Some("Apple M5")
        );
        assert_eq!(map.int("machdep.cpu.brand_string"), None);
        assert_eq!(Sysctl::get(&map, "hw.missing"), None);
        assert_eq!(
            map.keys_with_prefix("hw.optional.arm."),
            vec!["hw.optional.arm.FEAT_AES", "hw.optional.arm.FEAT_SHA3"]
        );
    }

    #[test]
    fn numeric_looking_strings_read_as_ints() {
        assert_eq!(SysctlValue::Str(" 42 ".into()).as_i64(), Some(42));
    }

    #[test]
    fn map_fs_lists_direct_children_once() {
        let fs = BTreeMap::from([
            (
                "/sys/devices/system/cpu/cpu0/topology/core_id".to_string(),
                "0".to_string(),
            ),
            (
                "/sys/devices/system/cpu/cpu0/online".to_string(),
                "1".to_string(),
            ),
            (
                "/sys/devices/system/cpu/cpu1/online".to_string(),
                "1".to_string(),
            ),
            (
                "/sys/devices/system/cpu/online".to_string(),
                "0-1".to_string(),
            ),
        ]);
        assert_eq!(
            fs.list("/sys/devices/system/cpu"),
            vec!["cpu0", "cpu1", "online"]
        );
        assert_eq!(
            fs.list("/sys/devices/system/cpu/"),
            vec!["cpu0", "cpu1", "online"]
        );
        assert_eq!(
            fs.read("/sys/devices/system/cpu/online").as_deref(),
            Some("0-1")
        );
    }

    #[test]
    fn os_names() {
        assert_eq!(Os::from_name("macos"), Os::MacOs);
        assert_eq!(Os::from_name("linux"), Os::Linux);
        assert_eq!(Os::from_name("windows"), Os::Other);
    }
}
