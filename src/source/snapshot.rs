//! The snapshot format: a directory or `.tar.gz` holding everything a collector reads, so any
//! machine can be replayed anywhere (`cpu --from` and every fixture test).
//!
//! Layout: `meta.toml`, `sysctl.toml` (flat `"key" = value`), and `fs/<absolute path>` files.

use std::collections::BTreeMap;
use std::fmt;
use std::fs;
use std::io::{self, Read};
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use serde::{Deserialize, Serialize};

use super::{Os, Sources, Stub, SysctlValue};

pub const SNAPSHOT_VERSION: u32 = 1;

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
        for (path, text) in &self.files {
            entries.insert(format!("fs{path}"), text.clone());
        }
        entries
    }

    pub fn write_tar_gz(&self, path: &Path) -> io::Result<()> {
        let file = fs::File::create(path)?;
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
            ioreg: Box::new(Stub),
        }
    }
}

fn invalid(file: &str, err: impl fmt::Display) -> SnapshotError {
    SnapshotError::Invalid {
        file: file.to_string(),
        message: err.to_string(),
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

fn read_dir_entries(root: &Path) -> io::Result<BTreeMap<String, String>> {
    fn walk(root: &Path, dir: &Path, out: &mut BTreeMap<String, String>) -> io::Result<()> {
        for entry in fs::read_dir(dir)? {
            let path = entry?.path();
            if path.is_dir() {
                walk(root, &path, out)?;
                continue;
            }
            let relative = path
                .strip_prefix(root)
                .expect("walked paths are under root");
            let name = relative
                .components()
                .map(|c| c.as_os_str().to_string_lossy())
                .collect::<Vec<_>>()
                .join("/");
            out.insert(
                name,
                String::from_utf8_lossy(&fs::read(&path)?).into_owned(),
            );
        }
        Ok(())
    }
    let mut out = BTreeMap::new();
    walk(root, root, &mut out)?;
    Ok(out)
}

fn read_tar_gz_entries(path: &Path) -> io::Result<BTreeMap<String, String>> {
    let decoder = flate2::read::GzDecoder::new(fs::File::open(path)?);
    let mut archive = tar::Archive::new(decoder);
    let mut out = BTreeMap::new();
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
        let mut bytes = Vec::new();
        entry.read_to_end(&mut bytes)?;
        out.insert(name, String::from_utf8_lossy(&bytes).into_owned());
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::source::Sysctl;

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

    #[test]
    fn recorded_sources_carry_the_snapshot_os() {
        let sources = sample().into_sources();
        assert_eq!(sources.os, Os::MacOs);
        assert_eq!(sources.sysctl.int("hw.ncpu"), Some(10));
    }
}
