//! Renderers: pure functions from a [`Cpu`] to text, plus the choice of which one to use.

use std::io::{self, IsTerminal, Write};

use crate::model::Cpu;

pub mod boxed;
pub mod explain;
pub mod json;
pub mod palette;
pub mod plain;
pub mod view;

use palette::Theme;

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
    /// How many colours the terminal claims.
    pub depth: Depth,
    /// `TERM=dumb`: no colour unless `--color always`.
    pub dumb: bool,
}

impl Terminal {
    pub fn detect() -> Terminal {
        let var = |name: &str| std::env::var(name).ok().filter(|v| !v.is_empty());
        let locale = var("LC_ALL")
            .or_else(|| var("LC_CTYPE"))
            .or_else(|| var("LANG"));
        let term = var("TERM");
        Terminal {
            stdout_is_tty: io::stdout().is_terminal(),
            utf8: locale_is_utf8(locale.as_deref()),
            no_color: var("NO_COLOR").is_some(),
            force_color: var("CLICOLOR_FORCE").is_some_and(|v| v != "0"),
            depth: Depth::from_env(var("COLORTERM").as_deref(), term.as_deref()),
            dumb: term.as_deref() == Some("dumb"),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Depth {
    Ansi16,
    Ansi256,
}

impl Depth {
    /// 256 colours when `COLORTERM` says truecolor/24bit or `TERM` names a 256-colour terminal.
    pub fn from_env(colorterm: Option<&str>, term: Option<&str>) -> Depth {
        if matches!(colorterm, Some("truecolor" | "24bit"))
            || term.is_some_and(|t| t.contains("256color"))
        {
            Depth::Ansi256
        } else {
            Depth::Ansi16
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Background {
    Dark,
    Light,
    Unknown,
}

/// Asks the terminal whether its background is light (OSC 10 and 11, fenced by DA1, so a terminal
/// without support answers at once). Any failure is `Unknown`. Background jobs never ask: changing
/// terminal settings from one would get it stopped by the shell (SIGTTOU).
pub fn query_background() -> Background {
    if !in_foreground() {
        return Background::Unknown;
    }
    match terminal_colorsaurus::theme_mode(query_options()) {
        Ok(terminal_colorsaurus::ThemeMode::Light) => Background::Light,
        Ok(terminal_colorsaurus::ThemeMode::Dark) => Background::Dark,
        Err(_) => Background::Unknown,
    }
}

/// The library default (1 s): a reply that arrives after the timeout would be read by the shell
/// instead, so a short timeout breaks slow links such as SSH.
fn query_options() -> terminal_colorsaurus::QueryOptions {
    terminal_colorsaurus::QueryOptions::default()
}

/// Whether this process is in the foreground process group of the terminal on stdout.
#[cfg(unix)]
fn in_foreground() -> bool {
    // SAFETY: both calls only read process state; no memory is shared with the kernel.
    let (terminal, own) = unsafe { (libc::tcgetpgrp(libc::STDOUT_FILENO), libc::getpgrp()) };
    terminal != -1 && terminal == own
}

#[cfg(not(unix))]
fn in_foreground() -> bool {
    false
}

/// An unset locale is assumed UTF-8 (macOS terminals); `C`/`POSIX` and other non-UTF-8 locales are not.
pub fn locale_is_utf8(locale: Option<&str>) -> bool {
    locale.is_none_or(|l| {
        let l = l.to_ascii_lowercase();
        l.contains("utf-8") || l.contains("utf8")
    })
}

/// Picks the output mode and, for coloured boxes, the palette. `--json` wins. Boxes need a UTF-8
/// locale and either a terminal or an explicit request for colour; everything else gets plain
/// text. `background` is called only when colour is on, the terminal has 256 colours and stdout is
/// a terminal that can be asked.
pub fn choose(
    json: bool,
    plain: bool,
    explain: bool,
    color: ColorChoice,
    term: &Terminal,
    background: impl FnOnce() -> Background,
) -> (Mode, Option<Theme>) {
    if json {
        return (Mode::Json, None);
    }
    if explain {
        return (Mode::Explain, None);
    }
    let color_on = match color {
        ColorChoice::Always => true,
        ColorChoice::Never => false,
        ColorChoice::Auto => {
            !term.no_color && !term.dumb && (term.force_color || term.stdout_is_tty)
        }
    };
    if plain || !term.utf8 || !(term.stdout_is_tty || color_on) {
        return (Mode::Plain, None);
    }
    if !color_on {
        return (Mode::Boxed, None);
    }
    let theme = match term.depth {
        Depth::Ansi16 => Theme::Ansi16,
        Depth::Ansi256 if term.stdout_is_tty && background() == Background::Light => {
            Theme::Light256
        }
        Depth::Ansi256 => Theme::Dark256,
    };
    (Mode::Boxed, Some(theme))
}

pub fn render(cpu: &Cpu, mode: Mode, theme: Option<Theme>) -> String {
    let palette = theme.map(Theme::palette);
    match mode {
        Mode::Json => json::render(cpu),
        Mode::Explain => explain::render(cpu),
        Mode::Plain => with_footnote(
            plain::render(&view::build(cpu, &view::ASCII)),
            view::footnote(cpu, &view::ASCII).map(|n| n.to_string()),
        ),
        Mode::Boxed => with_footnote(
            boxed::render(&view::build(cpu, &view::UNICODE), palette),
            view::footnote(cpu, &view::UNICODE).map(|n| boxed::paint_line(&n, palette)),
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
            depth: Depth::Ansi256,
            dumb: false,
        }
    }

    /// For cases where the terminal must not be asked.
    fn unasked() -> Background {
        panic!("the terminal background was queried")
    }

    fn pick(color: ColorChoice, t: &Terminal) -> (Mode, Option<Theme>) {
        choose(false, false, false, color, t, unasked)
    }

    #[test]
    fn a_terminal_gets_the_palette_for_its_background() {
        let t = term(true, true);
        for (bg, theme) in [
            (Background::Dark, Theme::Dark256),
            (Background::Light, Theme::Light256),
            (Background::Unknown, Theme::Dark256),
        ] {
            assert_eq!(
                choose(false, false, false, ColorChoice::Auto, &t, || bg),
                (Mode::Boxed, Some(theme))
            );
        }
    }

    #[test]
    fn sixteen_colour_terminals_are_never_asked() {
        let t = Terminal {
            depth: Depth::Ansi16,
            ..term(true, true)
        };
        assert_eq!(
            pick(ColorChoice::Auto, &t),
            (Mode::Boxed, Some(Theme::Ansi16))
        );
    }

    #[test]
    fn pipes_get_plain() {
        assert_eq!(
            pick(ColorChoice::Auto, &term(false, true)),
            (Mode::Plain, None)
        );
    }

    #[test]
    fn json_always_wins() {
        assert_eq!(
            choose(
                true,
                true,
                false,
                ColorChoice::Always,
                &term(true, true),
                unasked
            ),
            (Mode::Json, None)
        );
    }

    #[test]
    fn plain_flag_on_a_terminal() {
        assert_eq!(
            choose(
                false,
                true,
                false,
                ColorChoice::Auto,
                &term(true, true),
                unasked
            ),
            (Mode::Plain, None)
        );
    }

    #[test]
    fn non_utf8_locale_gets_plain() {
        assert_eq!(
            pick(ColorChoice::Auto, &term(true, false)),
            (Mode::Plain, None)
        );
    }

    #[test]
    fn no_color_keeps_the_boxes() {
        let t = Terminal {
            no_color: true,
            ..term(true, true)
        };
        assert_eq!(pick(ColorChoice::Auto, &t), (Mode::Boxed, None));
    }

    #[test]
    fn forced_colour_into_a_pipe_is_never_asked() {
        assert_eq!(
            pick(ColorChoice::Always, &term(false, true)),
            (Mode::Boxed, Some(Theme::Dark256))
        );
        let t = Terminal {
            force_color: true,
            ..term(false, true)
        };
        assert_eq!(
            pick(ColorChoice::Auto, &t),
            (Mode::Boxed, Some(Theme::Dark256))
        );
    }

    #[test]
    fn color_never_on_a_terminal() {
        assert_eq!(
            pick(ColorChoice::Never, &term(true, true)),
            (Mode::Boxed, None)
        );
    }

    #[test]
    fn term_dumb_turns_colour_off_like_no_color() {
        let dumb = Terminal {
            dumb: true,
            ..term(true, true)
        };
        assert_eq!(pick(ColorChoice::Auto, &dumb), (Mode::Boxed, None));
        let forced = Terminal {
            force_color: true,
            ..dumb
        };
        assert_eq!(pick(ColorChoice::Auto, &forced), (Mode::Boxed, None));
        assert_eq!(
            choose(false, false, false, ColorChoice::Always, &dumb, || {
                Background::Unknown
            }),
            (Mode::Boxed, Some(Theme::Dark256))
        );
    }

    #[test]
    fn the_query_waits_long_enough_for_a_slow_link() {
        // A reply that arrives after the timeout lands in the user's shell, so wait as long as the
        // library recommends; terminals without support still answer at once.
        assert_eq!(query_options().timeout, std::time::Duration::from_secs(1));
    }

    #[test]
    fn depth_from_the_environment() {
        assert_eq!(Depth::from_env(Some("truecolor"), None), Depth::Ansi256);
        assert_eq!(
            Depth::from_env(Some("24bit"), Some("xterm")),
            Depth::Ansi256
        );
        assert_eq!(
            Depth::from_env(None, Some("xterm-256color")),
            Depth::Ansi256
        );
        assert_eq!(
            Depth::from_env(None, Some("screen-256color")),
            Depth::Ansi256
        );
        assert_eq!(Depth::from_env(None, Some("xterm")), Depth::Ansi16);
        assert_eq!(Depth::from_env(None, Some("linux")), Depth::Ansi16);
        assert_eq!(Depth::from_env(None, None), Depth::Ansi16);
    }

    #[test]
    fn explain_is_plain_text_and_json_still_wins() {
        assert_eq!(
            choose(
                false,
                false,
                true,
                ColorChoice::Always,
                &term(true, true),
                unasked
            ),
            (Mode::Explain, None)
        );
        assert_eq!(
            choose(
                true,
                false,
                true,
                ColorChoice::Auto,
                &term(true, true),
                unasked
            ),
            (Mode::Json, None)
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
}
