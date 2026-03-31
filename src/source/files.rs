//! Live file reads, for Linux's `/proc` and `/sys`. Paths are absolute; the root lets tests run
//! the same code against a temporary directory.

use std::fs;
use std::io::Read;
use std::path::PathBuf;

use super::Fs;

/// Largest file read. `/proc/cpuinfo` on a 256-CPU server is a few hundred KiB.
const MAX_FILE_BYTES: u64 = 4 << 20;

pub struct LiveFs {
    root: PathBuf,
}

impl LiveFs {
    /// The real filesystem.
    pub fn root() -> LiveFs {
        LiveFs::at("/")
    }

    /// Absolute paths resolved under `root`.
    pub fn at(root: impl Into<PathBuf>) -> LiveFs {
        LiveFs { root: root.into() }
    }

    fn resolve(&self, path: &str) -> PathBuf {
        self.root.join(path.trim_start_matches('/'))
    }
}

impl Fs for LiveFs {
    fn read(&self, path: &str) -> Option<String> {
        let mut bytes = Vec::new();
        fs::File::open(self.resolve(path))
            .ok()?
            .take(MAX_FILE_BYTES)
            .read_to_end(&mut bytes)
            .ok()?;
        Some(String::from_utf8_lossy(&bytes).into_owned())
    }

    fn list(&self, dir: &str) -> Vec<String> {
        let mut names: Vec<String> = fs::read_dir(self.resolve(dir))
            .map(|entries| {
                entries
                    .filter_map(Result::ok)
                    .map(|e| e.file_name().to_string_lossy().into_owned())
                    .collect()
            })
            .unwrap_or_default();
        names.sort();
        names
    }
}

#[cfg(test)]
mod tests {
    use std::fs;

    use super::*;

    fn tree() -> tempfile::TempDir {
        let dir = tempfile::tempdir().unwrap();
        fs::create_dir_all(dir.path().join("sys/devices/system/cpu/cpu1")).unwrap();
        fs::create_dir_all(dir.path().join("sys/devices/system/cpu/cpu0")).unwrap();
        fs::write(dir.path().join("sys/devices/system/cpu/online"), "0-1\n").unwrap();
        dir
    }

    #[test]
    fn reads_files_by_absolute_path_under_its_root() {
        let dir = tree();
        let fs = LiveFs::at(dir.path());
        assert_eq!(
            fs.read("/sys/devices/system/cpu/online").as_deref(),
            Some("0-1\n")
        );
        assert_eq!(fs.read("/sys/devices/system/cpu/missing"), None);
        assert_eq!(
            fs.read("/sys/devices/system/cpu"),
            None,
            "a directory is not a file"
        );
    }

    #[test]
    fn lists_children_sorted() {
        let dir = tree();
        let fs = LiveFs::at(dir.path());
        assert_eq!(
            fs.list("/sys/devices/system/cpu"),
            vec!["cpu0", "cpu1", "online"]
        );
        assert!(fs.list("/nope").is_empty());
    }
}
