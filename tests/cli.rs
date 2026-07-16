use std::path::PathBuf;
use std::process::{Command, Output};

fn cpu() -> Command {
    let mut cmd = Command::new(env!("CARGO_BIN_EXE_cpu"));
    cmd.env_remove("CLICOLOR_FORCE")
        .env_remove("NO_COLOR")
        .env_remove("LC_ALL")
        .env_remove("LC_CTYPE")
        .env("LANG", "en_US.UTF-8");
    cmd
}

fn fixture(name: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures")
        .join(name)
}

fn run(cmd: &mut Command) -> (i32, String, String) {
    let Output {
        status,
        stdout,
        stderr,
    } = cmd.output().expect("cpu runs");
    (
        status.code().expect("exited normally"),
        String::from_utf8(stdout).unwrap(),
        String::from_utf8(stderr).unwrap(),
    )
}

#[test]
fn piped_output_is_plain_ascii() {
    let (code, out, err) = run(cpu().arg("--from").arg(fixture("apple-m5")));
    assert_eq!(code, 0, "{err}");
    assert!(
        out.starts_with("Identity\n  Name          Apple M5\n"),
        "{out}"
    );
    assert!(out.is_ascii() && !out.contains('\x1b'), "{out}");
}

#[test]
fn color_always_gives_boxes_even_in_a_pipe() {
    let (code, out, _) = run(cpu()
        .args(["--color", "always", "--from"])
        .arg(fixture("apple-m5")));
    assert_eq!(code, 0);
    assert!(out.contains("╭") && out.contains("\x1b["), "{out}");
}

#[test]
fn json_output() {
    let (code, out, _) = run(cpu().arg("--json").arg("--from").arg(fixture("apple-m5")));
    assert_eq!(code, 0);
    let v: serde_json::Value = serde_json::from_str(&out).unwrap();
    assert_eq!(v["schema_version"], 1);
}

#[test]
fn missing_snapshot_exits_1() {
    let (code, out, err) = run(cpu().args(["--from", "/definitely/not/here"]));
    assert_eq!(code, 1);
    assert!(out.is_empty());
    assert!(
        err.starts_with("cpu: cannot read snapshot /definitely/not/here"),
        "{err}"
    );
}

#[test]
fn newer_snapshot_version_exits_1() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(dir.path().join("meta.toml"), "snapshot_version = 2\n").unwrap();
    let (code, _, err) = run(cpu().arg("--from").arg(dir.path()));
    assert_eq!(code, 1);
    assert!(err.contains("unsupported snapshot version 2"), "{err}");
}

#[test]
fn unknown_flag_exits_2() {
    let (code, _, _) = run(cpu().arg("--bogus"));
    assert_eq!(code, 2);
}

#[test]
fn dump_writes_a_snapshot_that_opens() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("me.tar.gz");
    let (code, out, err) = run(cpu().arg("--dump").arg(&path));
    assert_eq!(code, 0, "{err}");
    assert_eq!(out.trim(), path.display().to_string());
    let snap = cpu_cli::source::Snapshot::open(&path).unwrap();
    assert_eq!(snap.meta.os, std::env::consts::OS);
}

#[test]
fn dump_and_from_conflict() {
    let (code, _, err) = run(cpu().args(["--dump", "x.tar.gz", "--from", "y"]));
    assert_eq!(code, 2);
    assert!(err.contains("cannot be used with"), "{err}");
}

#[cfg(any(target_os = "macos", target_os = "linux"))]
#[test]
fn a_dump_replays_exactly_like_the_live_machine() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("me.tar.gz");
    assert_eq!(run(cpu().arg("--dump").arg(&path)).0, 0);
    let (_, live, _) = run(cpu().arg("--json"));
    let (_, replayed, _) = run(cpu().arg("--json").arg("--from").arg(&path));
    assert_eq!(live, replayed);
}

#[test]
fn hostile_snapshot_values_never_reach_the_terminal() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(
        dir.path().join("meta.toml"),
        "snapshot_version = 1\ncpu_version = \"0.1.0\"\nos = \"macos\"\narch = \"aarch64\"\ncreated_unix = 0\n",
    )
    .unwrap();
    std::fs::write(
        dir.path().join("sysctl.toml"),
        "\"machdep.cpu.brand_string\" = \"Apple \\u001b]0;pwned\\u0007M5\"\n\"hw.logicalcpu\" = 8\n",
    )
    .unwrap();
    for flags in [
        &["--plain"][..],
        &["--color", "always"][..],
        &["--json"][..],
        &["--explain"][..],
    ] {
        let (code, out, err) = run(cpu().args(flags).arg("--from").arg(dir.path()));
        assert_eq!(code, 0, "{err}");
        assert!(
            !out.contains('\x1b') || flags == ["--color", "always"],
            "{flags:?}: {out:?}"
        );
        assert!(!out.contains("pwned"), "{flags:?}: {out:?}");
    }
}

#[test]
fn hostile_snapshot_errors_never_reach_the_terminal() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(
        dir.path().join("meta.toml"),
        "snapshot_version = 1\ncpu_version = \"0.1.0\"\nos = \"macos\"\narch = \"aarch64\"\ncreated_unix = 0\n",
    )
    .unwrap();
    std::fs::write(
        dir.path().join("ioreg.toml"),
        "\"pmgr:\\u001b]0;pwned\\u0007\" = \"zz\"\n",
    )
    .unwrap();
    let (code, _, err) = run(cpu().arg("--from").arg(dir.path()));
    assert_ne!(code, 0);
    assert!(err.contains("not hex"), "{err:?}");
    assert!(!err.contains(['\x1b', '\x07']), "{err:?}");
}

#[test]
fn explain_lists_origins() {
    let (code, out, _) = run(cpu()
        .arg("--explain")
        .arg("--from")
        .arg(fixture("apple-m5")));
    assert_eq!(code, 0);
    assert!(out.contains("sysctl:hw.perflevel0.l2cachesize"), "{out}");
}

#[test]
fn explain_conflicts_with_json() {
    let (code, _, err) = run(cpu()
        .args(["--explain", "--json", "--from"])
        .arg(fixture("apple-m5")));
    assert_eq!(code, 2);
    assert!(err.contains("cannot be used with"), "{err}");
}

#[test]
fn dump_refuses_to_overwrite() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("taken.tar.gz");
    std::fs::write(&path, "precious").unwrap();
    let (code, _, err) = run(cpu().arg("--dump").arg(&path));
    assert_eq!(code, 1);
    assert!(err.contains("already exists"), "{err}");
    assert_eq!(std::fs::read_to_string(&path).unwrap(), "precious");
}

#[cfg(unix)]
#[test]
fn dump_never_writes_through_a_dangling_symlink() {
    let dir = tempfile::tempdir().unwrap();
    let link = dir.path().join("dump.tar.gz");
    let target = dir.path().join("elsewhere");
    std::os::unix::fs::symlink(&target, &link).unwrap();
    let (code, _, err) = run(cpu().arg("--dump").arg(&link));
    assert_eq!(code, 1);
    assert!(err.contains("already exists"), "{err}");
    assert!(!target.exists(), "the dump followed the symlink");
}

#[test]
fn dump_survives_a_closed_stdout() {
    use std::process::Stdio;
    let dir = tempfile::tempdir().unwrap();
    let mut child = cpu()
        .arg("--dump")
        .arg(dir.path().join("d.tar.gz"))
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    drop(child.stdout.take());
    let out = child.wait_with_output().unwrap();
    assert_eq!(
        out.status.code(),
        Some(0),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
}

#[test]
fn a_panic_asks_for_a_dump() {
    let (code, _, err) = run(cpu().env("CPU_TEST_PANIC", "1").arg("--plain"));
    assert_eq!(code, 101);
    assert!(
        err.contains("this is a bug") && err.contains("cpu --dump"),
        "{err}"
    );
}

#[test]
fn an_unidentifiable_snapshot_says_so() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(dir.path().join("meta.toml"), "snapshot_version = 1\ncpu_version = \"0.1.0\"\nos = \"macos\"\narch = \"aarch64\"\ncreated_unix = 0\n").unwrap();
    let (code, _, err) = run(cpu().arg("--from").arg(dir.path()));
    assert_eq!(code, 1);
    assert!(
        err.contains("does not identify a CPU") && !err.contains("--dump"),
        "{err}"
    );
}

#[test]
fn snapshot_errors_are_one_line() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(dir.path().join("meta.toml"), "snapshot_version = [\n").unwrap();
    let (_, _, err) = run(cpu().arg("--from").arg(dir.path()));
    assert_eq!(err.trim_end().lines().count(), 1, "{err}");
    assert!(err.contains("line 1"), "where it failed: {err}");
    std::fs::write(dir.path().join("meta.toml"), "snapshot_version = 1\n").unwrap();
    let (_, _, err) = run(cpu().arg("--from").arg(dir.path()));
    assert_eq!(err.trim_end().lines().count(), 1, "{err}");
    assert!(err.contains("missing field"), "why it failed: {err}");
}

#[test]
fn a_wrapped_tarball_opens() {
    let dir = tempfile::tempdir().unwrap();
    let archive = dir.path().join("wrapped.tar.gz");
    let status = Command::new("tar")
        .env("COPYFILE_DISABLE", "1") // no macOS ._ metadata files
        .arg("-czf")
        .arg(&archive)
        .arg("-C")
        .arg(fixture(""))
        .arg("apple-m5")
        .status()
        .unwrap();
    assert!(status.success());
    let (code, out, err) = run(cpu().arg("--plain").arg("--from").arg(&archive));
    assert_eq!(code, 0, "{err}");
    assert!(out.contains("Apple M5"));
}
