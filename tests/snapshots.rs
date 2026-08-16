mod common;

use cpu_cli::render::palette::Theme;
use cpu_cli::render::{Mode, render};

/// Every fixture in every output mode. New fixtures are picked up automatically.
#[test]
fn golden_outputs() {
    for (name, path) in common::fixtures() {
        let cpu = common::load(&path);
        for (kind, mode) in [
            ("boxed", Mode::Boxed),
            ("plain", Mode::Plain),
            ("json", Mode::Json),
        ] {
            let mut settings = insta::Settings::clone_current();
            settings.set_snapshot_suffix(format!("{name}-{kind}"));
            settings.bind(|| insta::assert_snapshot!(render(&cpu, mode, None)));
        }
    }
}

/// Colour output for two fixtures in each palette. Escapes are written as `\e` so the snapshot
/// reads as text in review.
#[test]
fn colour_golden_outputs() {
    for name in ["apple-m5", "linux-x86-intel-hybrid"] {
        let cpu = common::load(&common::fixture_dir().join(name));
        for (label, theme) in [
            ("dark256", Theme::Dark256),
            ("light256", Theme::Light256),
            ("ansi16", Theme::Ansi16),
        ] {
            let mut settings = insta::Settings::clone_current();
            settings.set_snapshot_suffix(format!("{name}-boxed-{label}"));
            let text = render(&cpu, Mode::Boxed, Some(theme)).replace('\x1b', "\\e");
            settings.bind(|| insta::assert_snapshot!(text));
        }
    }
}
