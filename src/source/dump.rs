//! `cpu --dump`: capture this machine's raw CPU data as a snapshot.
//!
//! Capture is an allowlist sweep, not "whatever the collector reads today", so old snapshots keep
//! working as collectors learn to read more. The allowlist is also the privacy guarantee: nothing
//! outside it (hostname, serial numbers, UUIDs) is ever written.

use std::collections::BTreeMap;
use std::io;

use super::SysctlValue;
use super::snapshot::{Meta, Snapshot};

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

pub fn capture_live() -> io::Result<Snapshot> {
    let sysctl = capture_sysctl()?;
    let kernel = match sysctl.get("kern.osrelease") {
        Some(SysctlValue::Str(s)) => Some(s.clone()),
        _ => None,
    };
    Ok(Snapshot {
        meta: Meta::current(kernel),
        sysctl,
        files: BTreeMap::new(),
    })
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
}
