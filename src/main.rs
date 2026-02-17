use std::io;
use std::path::PathBuf;
use std::process::ExitCode;

use clap::Parser;
use cpu_cli::collect::collect;
use cpu_cli::render::{self, ColorChoice, Terminal};
use cpu_cli::source::{Os, Sources};

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

    /// Show a saved snapshot (directory or .tar.gz) instead of this machine
    #[arg(long, value_name = "SNAPSHOT")]
    from: Option<PathBuf>,

    /// When to use colour
    #[arg(long, value_enum, default_value_t = ColorChoice::Auto)]
    color: ColorChoice,
}

fn main() -> ExitCode {
    let args = Args::parse();
    match run(&args) {
        Ok(()) => ExitCode::SUCCESS,
        Err(message) => {
            eprintln!("cpu: {message}");
            ExitCode::FAILURE
        }
    }
}

fn run(args: &Args) -> Result<(), String> {
    let sources = match &args.from {
        Some(path) => Sources::recorded(path).map_err(|e| e.to_string())?,
        None => Sources::live(),
    };
    if sources.os == Os::Other {
        return Err("this operating system is not supported yet".to_string());
    }
    let cpu = collect(&sources);
    if !cpu.is_identified() {
        return Err(
            "could not identify this CPU; please run `cpu --dump` and attach the file to a GitHub issue"
                .to_string(),
        );
    }
    let (mode, color) = render::choose(args.json, args.plain, args.color, &Terminal::detect());
    render::write_output(&mut io::stdout().lock(), &render::render(&cpu, mode, color))
        .map_err(|e| format!("could not write output: {e}"))
}
