use std::io;
use std::path::{Path, PathBuf};
use std::process::ExitCode;

use clap::Parser;
use cpu_cli::collect::collect;
use cpu_cli::render::{self, ColorChoice, Terminal};
use cpu_cli::source::{Os, Snapshot, Sources, dump};

/// Show what CPU you're actually running.
#[derive(Parser)]
#[command(name = "cpu", version)]
struct Args {
    /// Print machine-readable JSON (schema_version 1)
    #[arg(long)]
    json: bool,

    /// Print aligned plain text: no boxes, no colour, ASCII only
    #[arg(long)]
    plain: bool,

    /// List every value with where it came from, then anything that was rejected
    #[arg(long, conflicts_with_all = ["json", "plain"])]
    explain: bool,

    /// Show a saved snapshot (directory or .tar.gz) instead of this machine
    #[arg(long, value_name = "SNAPSHOT")]
    from: Option<PathBuf>,

    /// Save this machine's raw CPU data to FILE for a bug report (default: ./cpu-dump-<UTC time>.tar.gz)
    #[arg(long, value_name = "FILE", conflicts_with = "from")]
    dump: Option<Option<PathBuf>>,

    /// When to use colour
    #[arg(long, value_enum, default_value_t = ColorChoice::Auto)]
    color: ColorChoice,
}

/// The issue form that asks for a `cpu --dump` file; the repository comes from Cargo.toml.
const REPORT_URL: &str = concat!(
    env!("CARGO_PKG_REPOSITORY"),
    "/issues/new?template=machine-snapshot.md"
);

fn main() -> ExitCode {
    std::panic::set_hook(Box::new(|info| {
        eprintln!("cpu: internal error: {}", printable(&info.to_string()));
        eprintln!(
            "cpu: this is a bug; please run `cpu --dump` and attach the file to an issue: {REPORT_URL}"
        );
    }));
    #[cfg(debug_assertions)]
    if std::env::var_os("CPU_TEST_PANIC").is_some() {
        panic!("CPU_TEST_PANIC is set");
    }
    let args = Args::parse();
    match run(&args) {
        Ok(()) => ExitCode::SUCCESS,
        Err(message) => {
            eprintln!("cpu: {}", printable(&message));
            ExitCode::FAILURE
        }
    }
}

/// Error text can quote snapshot contents (a key, an archive entry name), which are untrusted:
/// control characters would let a snapshot drive the terminal, so they are shown as `?`. Line
/// breaks become spaces so every message stays on one line.
fn printable(message: &str) -> String {
    message
        .chars()
        .map(|c| match c {
            '\n' | '\r' | '\t' => ' ',
            c if c.is_control() => '?',
            c => c,
        })
        .collect()
}

fn run(args: &Args) -> Result<(), String> {
    if let Some(target) = &args.dump {
        return write_dump(target.as_deref());
    }
    let (sources, os_name) = match &args.from {
        Some(path) => {
            let snapshot = Snapshot::open(path).map_err(|e| e.to_string())?;
            let os = snapshot.meta.os.clone();
            (snapshot.into_sources(), os)
        }
        None => (Sources::live(), std::env::consts::OS.to_string()),
    };
    if sources.os == Os::Other {
        return Err(format!(
            "{os_name} is not supported yet; see {}/issues",
            env!("CARGO_PKG_REPOSITORY")
        ));
    }
    let cpu = collect(&sources);
    if !cpu.is_identified() {
        return Err(match &args.from {
            Some(path) => format!("{} does not identify a CPU", path.display()),
            None => format!(
                "could not identify this CPU; please run `cpu --dump` and attach the file to an issue: {REPORT_URL}"
            ),
        });
    }
    let (mode, color) = render::choose(
        args.json,
        args.plain,
        args.explain,
        args.color,
        &Terminal::detect(),
    );
    render::write_output(&mut io::stdout().lock(), &render::render(&cpu, mode, color))
        .map_err(|e| format!("could not write output: {e}"))
}

fn write_dump(target: Option<&Path>) -> Result<(), String> {
    let snapshot =
        dump::capture_live().map_err(|e| format!("could not capture this machine: {e}"))?;
    let path = target.map(Path::to_path_buf).unwrap_or_else(|| {
        PathBuf::from(format!(
            "cpu-dump-{}.tar.gz",
            utc_stamp(snapshot.meta.created_unix)
        ))
    });
    snapshot.write_tar_gz(&path).map_err(|e| match e.kind() {
        io::ErrorKind::AlreadyExists => {
            format!("{} already exists; choose another file", path.display())
        }
        _ => format!("could not write {}: {e}", path.display()),
    })?;
    render::write_output(&mut io::stdout().lock(), &format!("{}\n", path.display()))
        .map_err(|e| format!("could not write output: {e}"))?;
    eprintln!("cpu: saved CPU data only (no hostname, serial numbers or IDs)");
    Ok(())
}

/// `YYYYMMDD-HHMMSS` in UTC, for default dump names (civil-from-days, H. Hinnant).
fn utc_stamp(unix: u64) -> String {
    let (days, secs) = ((unix / 86_400) as i64, unix % 86_400);
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1_460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let day = doy - (153 * mp + 2) / 5 + 1;
    let month = if mp < 10 { mp + 3 } else { mp - 9 };
    let year = yoe + era * 400 + i64::from(month <= 2);
    format!(
        "{year:04}{month:02}{day:02}-{:02}{:02}{:02}",
        secs / 3_600,
        secs % 3_600 / 60,
        secs % 60
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn utc_stamps() {
        assert_eq!(utc_stamp(0), "19700101-000000");
        assert_eq!(utc_stamp(1_790_208_000), "20260924-000000");
        assert_eq!(utc_stamp(951_782_400 + 3_723), "20000229-010203");
    }
}
