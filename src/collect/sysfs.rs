//! Parsers for the text formats Linux uses in `/proc` and `/sys`.

use std::collections::BTreeMap;

/// Most CPUs one list may name. Guards against hostile ranges such as `0-4294967295`.
const MAX_CPUS: usize = 1 << 16;

/// Parses a CPU list such as `0-3,8,10-11` into sorted, de-duplicated CPU numbers.
/// An empty list is valid (`offline` is usually empty); malformed text is `None`.
pub fn parse_cpu_list(text: &str) -> Option<Vec<u32>> {
    let mut cpus = Vec::new();
    for part in text.trim().split(',').filter(|p| !p.trim().is_empty()) {
        let (first, last) = match part.split_once('-') {
            Some((a, b)) => (a.trim().parse::<u32>().ok()?, b.trim().parse::<u32>().ok()?),
            None => {
                let n = part.trim().parse::<u32>().ok()?;
                (n, n)
            }
        };
        if first > last || cpus.len() + (last - first) as usize >= MAX_CPUS {
            return None;
        }
        cpus.extend(first..=last);
    }
    cpus.sort_unstable();
    cpus.dedup();
    Some(cpus)
}

/// Parses a sysfs size such as `48K`, `2M` or `512` into bytes. Suffixes are binary.
pub fn parse_size(text: &str) -> Option<u64> {
    let text = text.trim();
    let split = text
        .find(|c: char| !c.is_ascii_digit())
        .unwrap_or(text.len());
    let (digits, unit) = text.split_at(split);
    let n: u64 = digits.parse().ok()?;
    let scale: u64 = match unit {
        "" => 1,
        "K" => 1 << 10,
        "M" => 1 << 20,
        "G" => 1 << 30,
        _ => return None,
    };
    n.checked_mul(scale)
}

/// Splits `/proc/cpuinfo` into one map per processor block (`field -> value`, both trimmed).
pub fn parse_cpuinfo(text: &str) -> Vec<BTreeMap<String, String>> {
    let mut blocks = Vec::new();
    let mut block = BTreeMap::new();
    for line in text.lines() {
        if line.trim().is_empty() {
            if !block.is_empty() {
                blocks.push(std::mem::take(&mut block));
            }
            continue;
        }
        if let Some((key, value)) = line.split_once(':') {
            block.insert(key.trim().to_string(), value.trim().to_string());
        }
    }
    if !block.is_empty() {
        blocks.push(block);
    }
    blocks
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cpu_lists() {
        assert_eq!(
            parse_cpu_list("0-3,8,10-11"),
            Some(vec![0, 1, 2, 3, 8, 10, 11])
        );
        assert_eq!(parse_cpu_list(" 0-1\n"), Some(vec![0, 1]));
        assert_eq!(parse_cpu_list("1,9"), Some(vec![1, 9]));
        assert_eq!(parse_cpu_list(""), Some(vec![]));
        assert_eq!(parse_cpu_list("3-1"), None);
        assert_eq!(parse_cpu_list("0-4294967295"), None);
        assert_eq!(parse_cpu_list("x"), None);
    }

    #[test]
    fn sizes() {
        assert_eq!(parse_size("48K"), Some(49_152));
        assert_eq!(parse_size("1280K"), Some(1_310_720));
        assert_eq!(parse_size("2M"), Some(2_097_152));
        assert_eq!(parse_size("512"), Some(512));
        assert_eq!(parse_size(""), None);
        assert_eq!(parse_size("12Q"), None);
        assert_eq!(parse_size("18446744073709551615K"), None);
    }

    #[test]
    fn cpuinfo_blocks() {
        let text = "processor\t: 0\nCPU implementer\t: 0x61\nCPU architecture: 8\n\nprocessor\t: 1\nCPU part\t: 0x000\n";
        let blocks = parse_cpuinfo(text);
        assert_eq!(blocks.len(), 2);
        assert_eq!(blocks[0]["CPU implementer"], "0x61");
        assert_eq!(blocks[0]["CPU architecture"], "8");
        assert_eq!(blocks[1]["processor"], "1");
        assert!(parse_cpuinfo("").is_empty());
    }
}
