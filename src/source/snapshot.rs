//! The snapshot format: a directory or `.tar.gz` holding everything a collector reads, so any
//! machine can be replayed anywhere (`cpu --from` and every fixture test).
//!
//! Layout: `meta.toml`, `sysctl.toml` (flat `"key" = value`), `ioreg.toml` (hex strings keyed
//! `service:key`), and `fs/<absolute path>` files.

use std::collections::BTreeMap;
use std::fmt;
use std::fs;
use std::io::{self, Read};
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use serde::{Deserialize, Serialize};

use super::{Os, Sources, Stub, SysctlValue};

pub const SNAPSHOT_VERSION: u32 = 1;

/// Largest single file a snapshot may contain. Real inputs are a few KiB; this stops a hostile or
/// mistaken `--from` (a gzip bomb, `--from ~/Downloads`) from exhausting memory.
const MAX_ENTRY_BYTES: u64 = 16 << 20;
/// Largest total a snapshot may contain.
const MAX_TOTAL_BYTES: u64 = 64 << 20;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Meta {
    pub snapshot_version: u32,
    pub cpu_version: String,
    /// `std::env::consts::OS` of the captured machine.
    pub os: String,
    pub arch: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub kernel: Option<String>,
    pub created_unix: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub note: Option<String>,
}

impl Meta {
    /// Metadata for the machine this binary is running on, now.
    pub fn current(kernel: Option<String>) -> Meta {
        Meta {
            snapshot_version: SNAPSHOT_VERSION,
            cpu_version: env!("CARGO_PKG_VERSION").to_string(),
            os: std::env::consts::OS.to_string(),
            arch: std::env::consts::ARCH.to_string(),
            kernel,
            created_unix: SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .map(|d| d.as_secs())
                .unwrap_or(0),
            note: None,
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct Snapshot {
    pub meta: Meta,
    pub sysctl: BTreeMap<String, SysctlValue>,
    /// File contents keyed by absolute path, e.g. `/proc/cpuinfo`.
    pub files: BTreeMap<String, String>,
    /// IOKit data properties keyed `service:key`.
    pub ioreg: BTreeMap<String, Vec<u8>>,
}

#[derive(Debug)]
pub enum SnapshotError {
    Io { path: PathBuf, source: io::Error },
    MissingMeta,
    Invalid { file: String, message: String },
    UnsupportedVersion(u32),
}

impl fmt::Display for SnapshotError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            SnapshotError::Io { path, source } => {
                write!(f, "cannot read snapshot {}: {source}", path.display())
            }
            SnapshotError::MissingMeta => write!(f, "not a cpu snapshot (meta.toml is missing)"),
            SnapshotError::Invalid { file, message } => {
                write!(f, "invalid {file} in snapshot: {message}")
            }
            SnapshotError::UnsupportedVersion(v) => write!(
                f,
                "unsupported snapshot version {v} (this cpu reads version {SNAPSHOT_VERSION})"
            ),
        }
    }
}

impl std::error::Error for SnapshotError {}

impl Snapshot {
    /// Opens a snapshot directory, or a `.tar.gz` written by [`Snapshot::write_tar_gz`].
    pub fn open(path: &Path) -> Result<Snapshot, SnapshotError> {
        let entries = if path.is_dir() {
            read_dir_entries(path)
        } else {
            read_tar_gz_entries(path)
        };
        let entries = entries.map_err(|source| SnapshotError::Io {
            path: path.to_path_buf(),
            source,
        })?;
        Snapshot::from_entries(&entries)
    }

    /// Parses snapshot files keyed by their path inside the snapshot (`meta.toml`, `fs/proc/cpuinfo`).
    pub fn from_entries(entries: &BTreeMap<String, String>) -> Result<Snapshot, SnapshotError> {
        let meta_text = entries.get("meta.toml").ok_or(SnapshotError::MissingMeta)?;
        // Read the version on its own first, so a future format fails with a clear message
        // instead of a confusing "missing field" error.
        #[derive(Deserialize)]
        struct VersionOnly {
            snapshot_version: u32,
        }
        let version: VersionOnly =
            toml::from_str(meta_text).map_err(|e| invalid("meta.toml", e))?;
        if version.snapshot_version != SNAPSHOT_VERSION {
            return Err(SnapshotError::UnsupportedVersion(version.snapshot_version));
        }
        let meta: Meta = toml::from_str(meta_text).map_err(|e| invalid("meta.toml", e))?;
        let sysctl = match entries.get("sysctl.toml") {
            Some(text) => parse_sysctl(text)?,
            None => BTreeMap::new(),
        };
        let ioreg = match entries.get("ioreg.toml") {
            Some(text) => parse_ioreg(text)?,
            None => BTreeMap::new(),
        };
        let files = entries
            .iter()
            .filter_map(|(name, text)| {
                name.strip_prefix("fs/")
                    .map(|p| (format!("/{p}"), text.clone()))
            })
            .collect();
        Ok(Snapshot {
            meta,
            sysctl,
            files,
            ioreg,
        })
    }

    /// The inverse of [`Snapshot::from_entries`].
    pub fn to_entries(&self) -> BTreeMap<String, String> {
        let mut entries = BTreeMap::new();
        entries.insert(
            "meta.toml".to_string(),
            toml::to_string(&self.meta).expect("meta is plain data"),
        );
        if !self.sysctl.is_empty() {
            let mut table = toml::Table::new();
            for (key, value) in &self.sysctl {
                let value = match value {
                    SysctlValue::Int(n) => toml::Value::Integer(*n),
                    SysctlValue::Str(s) => toml::Value::String(s.clone()),
                };
                table.insert(key.clone(), value);
            }
            entries.insert(
                "sysctl.toml".to_string(),
                toml::to_string(&table).expect("table is plain data"),
            );
        }
        if !self.ioreg.is_empty() {
            let mut table = toml::Table::new();
            for (key, bytes) in &self.ioreg {
                table.insert(
                    key.clone(),
                    toml::Value::String(super::ioreg::encode_hex(bytes)),
                );
            }
            entries.insert(
                "ioreg.toml".to_string(),
                toml::to_string(&table).expect("table is plain data"),
            );
        }
        for (path, text) in &self.files {
            entries.insert(format!("fs{path}"), text.clone());
        }
        entries
    }

    /// Fails with `AlreadyExists` if anything is at `path`, including a dangling symlink: the
    /// exclusive create is atomic, so a dump never overwrites a file.
    pub fn write_tar_gz(&self, path: &Path) -> io::Result<()> {
        let file = fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(path)?;
        let encoder = flate2::write::GzEncoder::new(file, flate2::Compression::default());
        let mut archive = tar::Builder::new(encoder);
        for (name, text) in self.to_entries() {
            let mut header = tar::Header::new_gnu();
            header.set_size(text.len() as u64);
            header.set_mode(0o644);
            header.set_mtime(self.meta.created_unix);
            archive.append_data(&mut header, name.as_str(), text.as_bytes())?;
        }
        archive.into_inner()?.finish()?;
        Ok(())
    }

    pub fn into_sources(self) -> Sources {
        Sources {
            os: Os::from_name(&self.meta.os),
            fs: Box::new(self.files),
            sysctl: Box::new(self.sysctl),
            cpuid: Box::new(Stub),
            ioreg: Box::new(self.ioreg),
        }
    }
}

fn invalid(file: &str, err: impl fmt::Display) -> SnapshotError {
    SnapshotError::Invalid {
        file: file.to_string(),
        // TOML errors span several lines around a caret diagram; keep the location and the
        // reason, drop the diagram, so the message fits on one CLI line.
        message: err
            .to_string()
            .lines()
            .map(str::trim)
            .filter(|l| {
                !l.is_empty() && !l.starts_with('|') && !l.contains(" | ") && !l.ends_with(" |")
            })
            .collect::<Vec<_>>()
            .join(": "),
    }
}

fn parse_sysctl(text: &str) -> Result<BTreeMap<String, SysctlValue>, SnapshotError> {
    let table: toml::Table = toml::from_str(text).map_err(|e| invalid("sysctl.toml", e))?;
    Ok(table
        .into_iter()
        .filter_map(|(key, value)| match value {
            toml::Value::Integer(n) => Some((key, SysctlValue::Int(n))),
            toml::Value::String(s) => Some((key, SysctlValue::Str(s))),
            _ => None,
        })
        .collect())
}

fn parse_ioreg(text: &str) -> Result<BTreeMap<String, Vec<u8>>, SnapshotError> {
    let table: toml::Table = toml::from_str(text).map_err(|e| invalid("ioreg.toml", e))?;
    table
        .into_iter()
        .filter_map(|(key, value)| value.as_str().map(|hex| (key, hex.to_string())))
        .map(|(key, hex)| match super::ioreg::decode_hex(&hex) {
            Some(bytes) => Ok((key, bytes)),
            None => Err(invalid("ioreg.toml", format!("{key} is not hex"))),
        })
        .collect()
}

/// Whether a path inside a snapshot is part of the format. Anything else is ignored unread.
fn in_layout(name: &str) -> bool {
    name == "meta.toml" || name == "sysctl.toml" || name == "ioreg.toml" || name.starts_with("fs/")
}

/// Reads one entry, refusing anything past the per-entry and running-total limits.
fn read_capped(reader: impl Read, name: &str, total: &mut u64) -> io::Result<String> {
    let too_large = |what: String| io::Error::new(io::ErrorKind::InvalidData, what);
    let mut bytes = Vec::new();
    reader.take(MAX_ENTRY_BYTES + 1).read_to_end(&mut bytes)?;
    if bytes.len() as u64 > MAX_ENTRY_BYTES {
        return Err(too_large(format!(
            "{name} is larger than {} MiB",
            MAX_ENTRY_BYTES >> 20
        )));
    }
    *total += bytes.len() as u64;
    if *total > MAX_TOTAL_BYTES {
        return Err(too_large(format!(
            "snapshot is larger than {} MiB",
            MAX_TOTAL_BYTES >> 20
        )));
    }
    Ok(String::from_utf8_lossy(&bytes).into_owned())
}

/// Reads only the snapshot layout, and only once `meta.toml` shows this is a snapshot, so a
/// mistaken `--from ~/Downloads` fails fast instead of reading the whole tree. Symlinks are
/// never followed.
fn read_dir_entries(root: &Path) -> io::Result<BTreeMap<String, String>> {
    fn walk(
        dir: &Path,
        prefix: &str,
        out: &mut BTreeMap<String, String>,
        total: &mut u64,
    ) -> io::Result<()> {
        for entry in fs::read_dir(dir)? {
            let entry = entry?;
            let kind = entry.file_type()?;
            let name = format!("{prefix}/{}", entry.file_name().to_string_lossy());
            if kind.is_dir() {
                walk(&entry.path(), &name, out, total)?;
            } else if kind.is_file() {
                let text = read_capped(fs::File::open(entry.path())?, &name, total)?;
                out.insert(name, text);
            }
        }
        Ok(())
    }
    let is_file = |name: &str| fs::symlink_metadata(root.join(name)).is_ok_and(|m| m.is_file());
    let mut out = BTreeMap::new();
    let mut total = 0;
    if !is_file("meta.toml") {
        return Ok(out);
    }
    for name in ["meta.toml", "sysctl.toml", "ioreg.toml"] {
        if is_file(name) {
            out.insert(
                name.to_string(),
                read_capped(fs::File::open(root.join(name))?, name, &mut total)?,
            );
        }
    }
    if fs::symlink_metadata(root.join("fs")).is_ok_and(|m| m.is_dir()) {
        walk(&root.join("fs"), "fs", &mut out, &mut total)?;
    }
    Ok(out)
}

fn read_tar_gz_entries(path: &Path) -> io::Result<BTreeMap<String, String>> {
    let decoder = flate2::read::GzDecoder::new(fs::File::open(path)?);
    let mut archive = tar::Archive::new(decoder);
    let mut out = BTreeMap::new();
    let mut total = 0;
    let mut root: Option<String> = None;
    for entry in archive.entries()? {
        let mut entry = entry?;
        if !entry.header().entry_type().is_file() {
            continue;
        }
        let name = entry
            .path()?
            .to_string_lossy()
            .trim_start_matches("./")
            .to_string();
        // A snapshot re-packed from its extracted directory has one top-level folder ("" is the
        // archive root). Files from a second folder would merge two machines into one CPU.
        let (folder, name) = if in_layout(&name) {
            (String::new(), name)
        } else {
            match name.split_once('/') {
                Some((folder, inner)) if in_layout(inner) => {
                    (folder.to_string(), inner.to_string())
                }
                _ => continue,
            }
        };
        match &root {
            None => root = Some(folder),
            Some(first) if *first != folder => {
                return Err(io::Error::new(
                    io::ErrorKind::InvalidData,
                    "the archive holds more than one snapshot",
                ));
            }
            Some(_) => {}
        }
        let text = read_capped(&mut entry, &name, &mut total)?;
        out.insert(name, text);
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample() -> Snapshot {
        Snapshot {
            meta: Meta {
                snapshot_version: 1,
                cpu_version: "0.1.0".into(),
                os: "macos".into(),
                arch: "aarch64".into(),
                kernel: Some("25.5.0".into()),
                created_unix: 1_790_208_000,
                note: None,
            },
            sysctl: BTreeMap::from([
                ("hw.ncpu".to_string(), SysctlValue::Int(10)),
                (
                    "machdep.cpu.brand_string".to_string(),
                    SysctlValue::Str("Apple M5".into()),
                ),
            ]),
            files: BTreeMap::from([("/proc/cpuinfo".to_string(), "processor\t: 0\n".to_string())]),
            ioreg: BTreeMap::from([(
                "pmgr:voltage-states5-sram".to_string(),
                vec![0x60, 0xf5, 0x13, 0x00, 0x16, 0x03, 0x00, 0x00],
            )]),
        }
    }

    #[test]
    fn entries_round_trip() {
        let entries = sample().to_entries();
        assert!(entries.contains_key("fs/proc/cpuinfo"));
        assert_eq!(Snapshot::from_entries(&entries).unwrap(), sample());
    }

    #[test]
    fn tar_gz_round_trip() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("snap.tar.gz");
        sample().write_tar_gz(&path).unwrap();
        assert_eq!(Snapshot::open(&path).unwrap(), sample());
    }

    #[test]
    fn directory_snapshots_open_too() {
        let dir = tempfile::tempdir().unwrap();
        for (name, text) in sample().to_entries() {
            let path = dir.path().join(name);
            fs::create_dir_all(path.parent().unwrap()).unwrap();
            fs::write(path, text).unwrap();
        }
        assert_eq!(Snapshot::open(dir.path()).unwrap(), sample());
    }

    #[test]
    fn newer_version_is_reported_even_if_other_fields_changed() {
        let entries = BTreeMap::from([(
            "meta.toml".to_string(),
            "snapshot_version = 2\nmachine = \"new\"\n".to_string(),
        )]);
        let err = Snapshot::from_entries(&entries).unwrap_err();
        assert!(matches!(err, SnapshotError::UnsupportedVersion(2)), "{err}");
        assert_eq!(
            err.to_string(),
            "unsupported snapshot version 2 (this cpu reads version 1)"
        );
    }

    #[test]
    fn missing_meta_is_not_a_snapshot() {
        let err = Snapshot::from_entries(&BTreeMap::new()).unwrap_err();
        assert!(matches!(err, SnapshotError::MissingMeta), "{err}");
    }

    #[test]
    fn a_file_that_is_not_gzip_is_an_io_error() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("notes.txt");
        fs::write(&path, "hello").unwrap();
        let err = Snapshot::open(&path).unwrap_err();
        assert!(matches!(err, SnapshotError::Io { .. }), "{err}");
    }

    #[test]
    fn a_missing_path_is_an_io_error() {
        let err = Snapshot::open(Path::new("/definitely/not/here")).unwrap_err();
        assert!(
            err.to_string()
                .starts_with("cannot read snapshot /definitely/not/here"),
            "{err}"
        );
    }

    fn write_raw_tar_gz(path: &Path, entries: &[(&str, Vec<u8>)]) {
        let file = fs::File::create(path).unwrap();
        let mut archive = tar::Builder::new(flate2::write::GzEncoder::new(
            file,
            flate2::Compression::fast(),
        ));
        for (name, data) in entries {
            let mut header = tar::Header::new_gnu();
            header.set_size(data.len() as u64);
            header.set_mode(0o644);
            archive
                .append_data(&mut header, name, data.as_slice())
                .unwrap();
        }
        archive.into_inner().unwrap().finish().unwrap();
    }

    #[cfg(unix)]
    #[test]
    fn a_directory_without_meta_is_rejected_without_walking_it() {
        use std::os::unix::fs::PermissionsExt;
        let dir = tempfile::tempdir().unwrap();
        let secret = dir.path().join("unreadable");
        fs::write(&secret, "x").unwrap();
        fs::set_permissions(&secret, fs::Permissions::from_mode(0o000)).unwrap();
        let err = Snapshot::open(dir.path()).unwrap_err();
        assert!(matches!(err, SnapshotError::MissingMeta), "{err}");
    }

    #[cfg(unix)]
    #[test]
    fn symlinks_inside_a_snapshot_are_not_followed() {
        let dir = tempfile::tempdir().unwrap();
        for (name, text) in sample().to_entries() {
            let path = dir.path().join(name);
            fs::create_dir_all(path.parent().unwrap()).unwrap();
            fs::write(path, text).unwrap();
        }
        std::os::unix::fs::symlink(".", dir.path().join("fs/loop")).unwrap();
        assert_eq!(Snapshot::open(dir.path()).unwrap(), sample());
    }

    #[test]
    fn oversized_entries_are_refused() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("big.tar.gz");
        let meta = sample().to_entries()["meta.toml"].clone().into_bytes();
        let big = vec![b'x'; (MAX_ENTRY_BYTES + 1) as usize];
        write_raw_tar_gz(&path, &[("meta.toml", meta), ("fs/proc/cpuinfo", big)]);
        let err = Snapshot::open(&path).unwrap_err();
        assert!(err.to_string().contains("larger than"), "{err}");
    }

    #[test]
    fn entries_outside_the_layout_are_ignored_unread() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("extra.tar.gz");
        let meta = sample().to_entries()["meta.toml"].clone().into_bytes();
        let junk = vec![b'x'; (MAX_ENTRY_BYTES + 1) as usize];
        write_raw_tar_gz(&path, &[("meta.toml", meta), ("junk.bin", junk)]);
        assert!(Snapshot::open(&path).is_ok());
    }

    #[test]
    fn an_archive_of_several_snapshots_is_refused() {
        let dir = tempfile::tempdir().unwrap();
        let meta = sample().to_entries()["meta.toml"].clone().into_bytes();
        let sysctl = sample().to_entries()["sysctl.toml"].clone().into_bytes();
        for entries in [
            vec![
                ("a/meta.toml", meta.clone()),
                ("b/sysctl.toml", sysctl.clone()),
            ],
            vec![
                ("meta.toml", meta.clone()),
                ("b/sysctl.toml", sysctl.clone()),
            ],
        ] {
            let path = dir.path().join("two.tar.gz");
            let _ = fs::remove_file(&path);
            write_raw_tar_gz(&path, &entries);
            let err = Snapshot::open(&path).unwrap_err();
            assert!(err.to_string().contains("more than one snapshot"), "{err}");
        }
    }

    #[test]
    fn recorded_sources_carry_the_snapshot_os() {
        let sources = sample().into_sources();
        assert_eq!(sources.os, Os::MacOs);
        assert_eq!(sources.sysctl.int("hw.ncpu"), Some(10));
    }

    #[test]
    fn ioreg_is_stored_as_hex() {
        let entries = sample().to_entries();
        assert_eq!(
            entries["ioreg.toml"].trim(),
            "\"pmgr:voltage-states5-sram\" = \"60f5130016030000\""
        );
    }
}
