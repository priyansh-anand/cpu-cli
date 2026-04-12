//! Collectors turn raw source data into the shared [`Cpu`] model.

pub mod features;
mod linux;
mod macos;
pub mod sysfs;

use crate::model::{Cpu, F, Fact};
use crate::source::{Os, Sources};

/// Largest plausible cache.
const MAX_CACHE_BYTES: u64 = 1 << 30;

pub fn collect(sources: &Sources) -> Cpu {
    match sources.os {
        Os::MacOs => macos::collect(sources.sysctl.as_ref()),
        Os::Linux => linux::collect(sources.fs.as_ref()),
        Os::Other => Cpu::default(),
    }
}

/// `a / b` as a derived fact, when both are known and divide evenly.
fn ratio(a: &F<u32>, b: &F<u32>) -> F<u32> {
    match (a, b) {
        (Some(a), Some(b)) if b.value > 0 && a.value % b.value == 0 => {
            Some(Fact::derived(a.value / b.value))
        }
        _ => None,
    }
}

/// A positive count as a derived fact.
fn count(n: usize) -> F<u32> {
    u32::try_from(n).ok().filter(|n| *n > 0).map(Fact::derived)
}

/// Real caches are whole KiB and far below 1 GiB. Not "power of two": the M5's efficiency L2 is 6 MiB.
fn plausible_cache_size(bytes: u64) -> bool {
    bytes > 0 && bytes % 1024 == 0 && bytes <= MAX_CACHE_BYTES
}
