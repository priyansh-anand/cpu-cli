//! Live `sysctl` reads on macOS.
//!
//! `sysctlbyname` returns untyped bytes, so each read first asks the kernel for the key's format
//! (the `{0, 4}` "oidfmt" query that `/usr/sbin/sysctl` itself uses) and decodes to match.

use std::ffi::CString;
use std::io;
use std::process::Command;
use std::ptr;

use super::{Sysctl, SysctlValue};

const MAX_MIB: usize = 12;
const CTLTYPE_MASK: u32 = 0xf;
const CTLTYPE_INT: u32 = 2;
const CTLTYPE_STRING: u32 = 3;
const CTLTYPE_QUAD: u32 = 4;

pub struct LiveSysctl;

impl Sysctl for LiveSysctl {
    fn get(&self, key: &str) -> Option<SysctlValue> {
        let mib = name_to_mib(key)?;
        let (kind, format) = oid_format(&mib)?;
        decode(kind, &format, &read(&mib)?)
    }

    fn keys_with_prefix(&self, prefix: &str) -> Vec<String> {
        list_names(&[prefix.trim_end_matches('.')])
            .unwrap_or_default()
            .into_iter()
            .filter(|k| k.starts_with(prefix))
            .collect()
    }
}

/// Every key under the given trees, e.g. `["hw", "machdep.cpu"]`, via `sysctl -N`.
pub fn list_names(trees: &[&str]) -> io::Result<Vec<String>> {
    let out = Command::new("/usr/sbin/sysctl")
        .arg("-N")
        .args(trees)
        .output()?;
    Ok(String::from_utf8_lossy(&out.stdout)
        .lines()
        .map(str::trim)
        .filter(|l| !l.is_empty())
        .map(String::from)
        .collect())
}

fn name_to_mib(key: &str) -> Option<Vec<libc::c_int>> {
    let name = CString::new(key).ok()?;
    let mut mib: [libc::c_int; MAX_MIB] = [0; MAX_MIB];
    let mut len = MAX_MIB;
    // SAFETY: `mib` has room for `len` entries and `name` is NUL-terminated.
    let rc = unsafe { libc::sysctlnametomib(name.as_ptr(), mib.as_mut_ptr(), &mut len) };
    (rc == 0).then(|| mib[..len].to_vec())
}

/// One `sysctl(3)` read. With `buf = None` it only asks how many bytes the value needs.
fn sysctl_read(mib: &[libc::c_int], buf: Option<&mut [u8]>) -> Option<usize> {
    let mut mib = mib.to_vec();
    let (out, mut len) = match buf {
        Some(b) => (b.as_mut_ptr().cast::<libc::c_void>(), b.len()),
        None => (ptr::null_mut(), 0),
    };
    // SAFETY: `out` is null (a size query) or points to `len` writable bytes.
    let rc = unsafe {
        libc::sysctl(
            mib.as_mut_ptr(),
            mib.len() as libc::c_uint,
            out,
            &mut len,
            ptr::null_mut(),
            0,
        )
    };
    (rc == 0).then_some(len)
}

fn oid_format(mib: &[libc::c_int]) -> Option<(u32, String)> {
    let query: Vec<libc::c_int> = [0, 4].into_iter().chain(mib.iter().copied()).collect();
    let mut buf = [0u8; 256];
    let len = sysctl_read(&query, Some(&mut buf))?;
    let kind = u32::from_ne_bytes(buf.get(..4)?.try_into().ok()?);
    let format = buf
        .get(4..len)?
        .split(|b| *b == 0)
        .next()
        .unwrap_or_default();
    Some((
        kind & CTLTYPE_MASK,
        String::from_utf8_lossy(format).into_owned(),
    ))
}

fn read(mib: &[libc::c_int]) -> Option<Vec<u8>> {
    let size = sysctl_read(mib, None)?;
    let mut buf = vec![0u8; size];
    let len = sysctl_read(mib, Some(&mut buf))?;
    buf.truncate(len);
    Some(buf)
}

/// Decodes by format string first (`A` string, `I`/`IU` 32-bit, `Q`/`L` 64-bit), falling back to
/// the type bits. Several packed integers become a space-separated string, as `sysctl` prints them.
fn decode(kind: u32, format: &str, bytes: &[u8]) -> Option<SysctlValue> {
    let width = match format.as_bytes().first() {
        Some(b'A') => return Some(SysctlValue::Str(c_string(bytes))),
        Some(b'I') => 4,
        Some(b'Q' | b'L') => 8,
        _ => match kind {
            CTLTYPE_STRING => return Some(SysctlValue::Str(c_string(bytes))),
            CTLTYPE_INT => 4,
            CTLTYPE_QUAD => 8,
            _ => return None,
        },
    };
    if bytes.is_empty() || bytes.len() % width != 0 {
        return None;
    }
    let unsigned = format.ends_with('U');
    let values: Vec<i64> = bytes
        .chunks_exact(width)
        .map(|c| match (width, unsigned) {
            (4, true) => i64::from(u32::from_ne_bytes([c[0], c[1], c[2], c[3]])),
            (4, false) => i64::from(i32::from_ne_bytes([c[0], c[1], c[2], c[3]])),
            _ => i64::from_ne_bytes([c[0], c[1], c[2], c[3], c[4], c[5], c[6], c[7]]),
        })
        .collect();
    match values.as_slice() {
        [one] => Some(SysctlValue::Int(*one)),
        many => Some(SysctlValue::Str(
            many.iter()
                .map(i64::to_string)
                .collect::<Vec<_>>()
                .join(" "),
        )),
    }
}

fn c_string(bytes: &[u8]) -> String {
    String::from_utf8_lossy(bytes.split(|b| *b == 0).next().unwrap_or_default()).into_owned()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_integers_and_strings_from_the_kernel() {
        assert!(LiveSysctl.int("hw.ncpu").is_some_and(|n| n > 0));
        assert!(
            LiveSysctl
                .string("machdep.cpu.brand_string")
                .is_some_and(|s| !s.is_empty())
        );
        assert_eq!(LiveSysctl.get("hw.definitely_not_a_key"), None);
    }

    #[test]
    fn array_values_become_space_separated_strings() {
        let Some(SysctlValue::Str(s)) = LiveSysctl.get("hw.cachesize") else {
            panic!("hw.cachesize should decode as an array");
        };
        assert!(s.split(' ').count() > 1, "{s}");
    }

    #[test]
    fn lists_keys_under_a_prefix() {
        let keys = LiveSysctl.keys_with_prefix("machdep.cpu.");
        assert!(
            keys.iter().any(|k| k == "machdep.cpu.brand_string"),
            "{keys:?}"
        );
        assert!(keys.iter().all(|k| k.starts_with("machdep.cpu.")));
    }

    #[test]
    fn decodes_by_format() {
        assert_eq!(
            decode(CTLTYPE_INT, "I", &(-3i32).to_ne_bytes()),
            Some(SysctlValue::Int(-3))
        );
        assert_eq!(
            decode(CTLTYPE_INT, "IU", &u32::MAX.to_ne_bytes()),
            Some(SysctlValue::Int(i64::from(u32::MAX)))
        );
        assert_eq!(
            decode(CTLTYPE_QUAD, "Q", &17_179_869_184i64.to_ne_bytes()),
            Some(SysctlValue::Int(17_179_869_184))
        );
        assert_eq!(
            decode(CTLTYPE_STRING, "A", b"Apple M5\0"),
            Some(SysctlValue::Str("Apple M5".into()))
        );
        let two: Vec<u8> = [1i64, 2].iter().flat_map(|n| n.to_ne_bytes()).collect();
        assert_eq!(decode(5, "Q", &two), Some(SysctlValue::Str("1 2".into())));
        assert_eq!(decode(CTLTYPE_INT, "I", &[1, 2, 3]), None);
        assert_eq!(decode(1, "N", &[]), None);
    }
}
