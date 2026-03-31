//! Collectors turn raw source data into the shared [`Cpu`] model.

pub mod features;
mod macos;
pub mod sysfs;

use crate::model::Cpu;
use crate::source::{Os, Sources};

pub fn collect(sources: &Sources) -> Cpu {
    match sources.os {
        Os::MacOs => macos::collect(sources.sysctl.as_ref()),
        Os::Linux | Os::Other => Cpu::default(),
    }
}
