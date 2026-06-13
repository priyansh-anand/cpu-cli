//! IOKit registry reads, through `/usr/sbin/ioreg`. Only the power manager node (`pmgr`) is read:
//! its `voltage-states*` data properties are each CPU cluster's frequency table.

use std::cell::OnceCell;
use std::process::Command;

use super::IoReg;

/// Live device-tree reads. The `pmgr` node is read once, on first use.
pub struct LiveIoReg {
    pmgr: OnceCell<Vec<(String, Vec<u8>)>>,
}

impl LiveIoReg {
    pub fn new() -> LiveIoReg {
        LiveIoReg {
            pmgr: OnceCell::new(),
        }
    }

    /// Every data property of `service`. Only `pmgr` is supported.
    pub fn properties(&self, service: &str) -> Vec<(String, Vec<u8>)> {
        if service != "pmgr" {
            return Vec::new();
        }
        self.pmgr.get_or_init(|| read_node("pmgr")).clone()
    }
}

impl Default for LiveIoReg {
    fn default() -> Self {
        LiveIoReg::new()
    }
}

impl IoReg for LiveIoReg {
    fn property(&self, service: &str, key: &str) -> Option<Vec<u8>> {
        self.properties(service)
            .into_iter()
            .find(|(k, _)| k == key)
            .map(|(_, v)| v)
    }
}

fn read_node(name: &str) -> Vec<(String, Vec<u8>)> {
    let args = [
        "-r",
        "-d",
        "1",
        "-l",
        "-w0",
        "-p",
        "IODeviceTree",
        "-n",
        name,
    ];
    match Command::new("/usr/sbin/ioreg").args(args).output() {
        Ok(out) => parse_properties(&String::from_utf8_lossy(&out.stdout)),
        Err(_) => Vec::new(),
    }
}

/// The `"name" = <0a1b...>` data properties in `ioreg -l` output. Other property kinds are skipped.
pub fn parse_properties(text: &str) -> Vec<(String, Vec<u8>)> {
    text.lines()
        .filter_map(|line| {
            let line = line.trim_start_matches([' ', '|']);
            let (key, rest) = line.strip_prefix('"')?.split_once("\" = <")?;
            Some((key.to_string(), decode_hex(rest.strip_suffix('>')?)?))
        })
        .collect()
}

pub fn decode_hex(hex: &str) -> Option<Vec<u8>> {
    if hex.len() % 2 != 0 {
        return None;
    }
    (0..hex.len())
        .step_by(2)
        .map(|i| u8::from_str_radix(hex.get(i..i + 2)?, 16).ok())
        .collect()
}

pub fn encode_hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}
