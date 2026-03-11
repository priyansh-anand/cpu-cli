mod common;

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
            settings.bind(|| insta::assert_snapshot!(render(&cpu, mode, false)));
        }
    }
}
