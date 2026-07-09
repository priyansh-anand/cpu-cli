//! Renderers: pure functions from a [`Cpu`] to text, plus the choice of which one to use.

use std::io::{self, IsTerminal, Write};

use crate::model::Cpu;

pub mod boxed;
pub mod explain;
pub mod json;
pub mod plain;
pub mod view;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Mode {
    Boxed,
    Plain,
    Json,
    Explain,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, clap::ValueEnum)]
pub enum ColorChoice {
    Auto,
    Always,
    Never,
}

/// What we know about where output is going.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Terminal {
    pub stdout_is_tty: bool,
    pub utf8: bool,
    /// `NO_COLOR` is set and non-empty (no-color.org).
    pub no_color: bool,
    /// `CLICOLOR_FORCE` is set, non-empty and not `0`.
    pub force_color: bool,
}

impl Terminal {
    pub fn detect() -> Terminal {
        let var = |name: &str| std::env::var(name).ok().filter(|v| !v.is_empty());
        let locale = var("LC_ALL")
            .or_else(|| var("LC_CTYPE"))
            .or_else(|| var("LANG"));
        Terminal {
            stdout_is_tty: io::stdout().is_terminal(),
            utf8: locale_is_utf8(locale.as_deref()),
            no_color: var("NO_COLOR").is_some(),
            force_color: var("CLICOLOR_FORCE").is_some_and(|v| v != "0"),
        }
    }
}

/// An unset locale is assumed UTF-8 (macOS terminals); `C`/`POSIX` and other non-UTF-8 locales are not.
pub fn locale_is_utf8(locale: Option<&str>) -> bool {
    locale.is_none_or(|l| {
        let l = l.to_ascii_lowercase();
        l.contains("utf-8") || l.contains("utf8")
    })
}

/// Picks the output mode and whether to colour it. `--json` wins. Boxes need a UTF-8 locale and
/// either a terminal or an explicit request for colour; everything else gets plain text.
pub fn choose(
    json: bool,
    plain: bool,
    explain: bool,
    color: ColorChoice,
    term: &Terminal,
) -> (Mode, bool) {
    if json {
        return (Mode::Json, false);
    }
    if explain {
        return (Mode::Explain, false);
    }
    let color_on = match color {
        ColorChoice::Always => true,
        ColorChoice::Never => false,
        ColorChoice::Auto => !term.no_color && (term.force_color || term.stdout_is_tty),
    };
    if plain || !term.utf8 || !(term.stdout_is_tty || color_on) {
        (Mode::Plain, false)
    } else {
        (Mode::Boxed, color_on)
    }
}

pub fn render(cpu: &Cpu, mode: Mode, color: bool) -> String {
    match mode {
        Mode::Json => json::render(cpu),
        Mode::Explain => explain::render(cpu),
        Mode::Plain => with_footnote(
            plain::render(&view::build(cpu, &view::ASCII)),
            view::footnote(cpu, &view::ASCII),
        ),
        Mode::Boxed => with_footnote(
            boxed::render(&view::build(cpu, &view::UNICODE), color),
            view::footnote(cpu, &view::UNICODE),
        ),
    }
}

fn with_footnote(mut text: String, note: Option<String>) -> String {
    if let Some(note) = note {
        text.push('\n');
        text.push_str(&note);
        text.push('\n');
    }
    text
}

/// Writes output, treating a closed pipe (`cpu | head -1`) as success rather than an error.
pub fn write_output(w: &mut impl Write, text: &str) -> io::Result<()> {
    match w.write_all(text.as_bytes()).and_then(|()| w.flush()) {
        Err(e) if e.kind() == io::ErrorKind::BrokenPipe => Ok(()),
        other => other,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn term(stdout_is_tty: bool, utf8: bool) -> Terminal {
        Terminal {
            stdout_is_tty,
            utf8,
            no_color: false,
            force_color: false,
        }
    }

    #[test]
    fn a_terminal_gets_colourful_boxes() {
        assert_eq!(
            choose(false, false, false, ColorChoice::Auto, &term(true, true)),
            (Mode::Boxed, true)
        );
    }

    #[test]
    fn pipes_get_plain() {
        assert_eq!(
            choose(false, false, false, ColorChoice::Auto, &term(false, true)),
            (Mode::Plain, false)
        );
    }

    #[test]
    fn json_always_wins() {
        assert_eq!(
            choose(true, true, false, ColorChoice::Always, &term(true, true)),
            (Mode::Json, false)
        );
    }

    #[test]
    fn plain_flag_on_a_terminal() {
        assert_eq!(
            choose(false, true, false, ColorChoice::Auto, &term(true, true)),
            (Mode::Plain, false)
        );
    }

    #[test]
    fn non_utf8_locale_gets_plain() {
        assert_eq!(
            choose(false, false, false, ColorChoice::Auto, &term(true, false)),
            (Mode::Plain, false)
        );
    }

    #[test]
    fn no_color_keeps_the_boxes() {
        let t = Terminal {
            no_color: true,
            ..term(true, true)
        };
        assert_eq!(
            choose(false, false, false, ColorChoice::Auto, &t),
            (Mode::Boxed, false)
        );
    }

    #[test]
    fn color_always_boxes_a_pipe() {
        assert_eq!(
            choose(false, false, false, ColorChoice::Always, &term(false, true)),
            (Mode::Boxed, true)
        );
    }

    #[test]
    fn clicolor_force_boxes_a_pipe() {
        let t = Terminal {
            force_color: true,
            ..term(false, true)
        };
        assert_eq!(
            choose(false, false, false, ColorChoice::Auto, &t),
            (Mode::Boxed, true)
        );
    }

    #[test]
    fn color_never_on_a_terminal() {
        assert_eq!(
            choose(false, false, false, ColorChoice::Never, &term(true, true)),
            (Mode::Boxed, false)
        );
    }

    #[test]
    fn locales() {
        assert!(locale_is_utf8(None));
        assert!(locale_is_utf8(Some("en_US.UTF-8")));
        assert!(locale_is_utf8(Some("C.utf8")));
        assert!(!locale_is_utf8(Some("C")));
        assert!(!locale_is_utf8(Some("POSIX")));
    }

    struct Failing(io::ErrorKind);

    impl Write for Failing {
        fn write(&mut self, _: &[u8]) -> io::Result<usize> {
            Err(self.0.into())
        }
        fn flush(&mut self) -> io::Result<()> {
            Ok(())
        }
    }

    #[test]
    fn closed_pipe_is_success() {
        assert!(write_output(&mut Failing(io::ErrorKind::BrokenPipe), "x").is_ok());
    }

    #[test]
    fn other_write_errors_are_reported() {
        assert!(write_output(&mut Failing(io::ErrorKind::PermissionDenied), "x").is_err());
    }

    #[test]
    fn explain_is_plain_text_and_json_still_wins() {
        assert_eq!(
            choose(false, false, true, ColorChoice::Always, &term(true, true)),
            (Mode::Explain, false)
        );
        assert_eq!(
            choose(true, false, true, ColorChoice::Auto, &term(true, true)),
            (Mode::Json, false)
        );
    }
}
