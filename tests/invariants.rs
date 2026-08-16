//! Rules every fixture must satisfy. They need no expected output, so they also protect new
//! contributor snapshots that nobody has reviewed line by line.

mod common;

use std::collections::BTreeSet;

use cpu_cli::render::palette::Theme;
use cpu_cli::render::view::{self, Body};
use cpu_cli::render::{Mode, render};
use unicode_width::UnicodeWidthStr;

#[test]
fn counts_add_up() {
    for (name, path) in common::fixtures() {
        let cpu = common::load(&path);
        let t = &cpu.topology;
        if let (Some(p), Some(l)) = (&t.physical_cores, &t.logical_cpus) {
            assert!(l.value >= p.value, "{name}: logical < physical");
        }
        let cores: Option<u32> = cpu
            .clusters
            .iter()
            .map(|c| c.cores.as_ref().map(|f| f.value))
            .sum();
        if let (Some(sum), Some(p)) = (cores, &t.physical_cores) {
            assert_eq!(
                sum, p.value,
                "{name}: cluster cores don't add up to physical cores"
            );
        }
        let threads: Option<u32> = cpu
            .clusters
            .iter()
            .map(|c| c.threads.as_ref().map(|f| f.value))
            .sum();
        if let (Some(sum), Some(l)) = (threads, &t.logical_cpus) {
            assert_eq!(
                sum, l.value,
                "{name}: cluster threads don't add up to logical CPUs"
            );
        }
        for cluster in &cpu.clusters {
            // Summed over every shape of a cache level, the CPUs covered must be the cluster's
            // threads: one instance must never stand in for different ones.
            let mut covered: std::collections::BTreeMap<
                (u8, cpu_cli::model::CacheKind),
                Option<u32>,
            > = std::collections::BTreeMap::new();
            for cache in &cluster.caches {
                let cpus = cache
                    .shared_by
                    .as_ref()
                    .zip(cache.instances.as_ref())
                    .map(|(s, i)| s.value * i.value);
                let entry = covered.entry((cache.level, cache.kind)).or_insert(Some(0));
                *entry = entry.zip(cpus).map(|(a, b)| a + b);
            }
            if let Some(t) = &cluster.threads {
                for ((level, _), sum) in covered {
                    if let Some(sum) = sum {
                        assert_eq!(
                            sum,
                            t.value,
                            "{name}: L{level} of {}: instances cover {sum} CPUs, cluster has {}",
                            cluster.label(),
                            t.value
                        );
                    }
                }
            }
        }
        if let Some(l) = &t.logical_cpus {
            for cache in &cpu.shared_caches {
                if let (Some(s), Some(i)) = (&cache.shared_by, &cache.instances) {
                    assert_eq!(
                        s.value * i.value,
                        l.value,
                        "{name}: shared L{}: shared_by x instances != logical CPUs",
                        cache.level
                    );
                }
            }
            if !t.numa_nodes.is_empty() {
                let numa: usize = t.numa_nodes.iter().map(|n| n.cpus.len()).sum();
                assert_eq!(
                    numa as u32, l.value,
                    "{name}: NUMA nodes don't cover every logical CPU"
                );
            }
        }
    }
}

#[test]
fn rendered_output_never_shows_unknowns() {
    for (name, path) in common::fixtures() {
        let cpu = common::load(&path);
        for mode in [Mode::Boxed, Mode::Plain] {
            let text = render(&cpu, mode, None);
            for bad in ["None", "Some(", " 0 B", " 0 Hz"] {
                assert!(
                    !text.contains(bad),
                    "{name} {mode:?} contains {bad:?}:\n{text}"
                );
            }
        }
        for section in view::build(&cpu, &view::UNICODE) {
            match section.body {
                Body::Pairs(pairs) => {
                    for pair in pairs {
                        assert!(
                            pair.lines.iter().all(|l| !l.trim().is_empty()),
                            "{name}: blank value for {}",
                            pair.label
                        );
                    }
                }
                Body::Grid(grid) => {
                    assert!(!grid.rows.is_empty(), "{name}: empty {}", section.title)
                }
            }
        }
    }
}

#[test]
fn boxed_lines_have_equal_width() {
    for (name, path) in common::fixtures() {
        let text = render(&common::load(&path), Mode::Boxed, None);
        let widths: BTreeSet<usize> = text
            .lines()
            .filter(|l| l.starts_with(['╭', '│', '├', '╰']))
            .map(UnicodeWidthStr::width)
            .collect();
        assert!(
            text.lines().all(|l| UnicodeWidthStr::width(l) <= 80),
            "{name}: a line is wider than 80 columns:\n{text}"
        );
        assert_eq!(widths.len(), 1, "{name}:\n{text}");
        assert!(
            widths.iter().all(|w| *w <= 80),
            "{name}: wider than an 80-column terminal:\n{text}"
        );
    }
}

#[test]
fn plain_output_is_ascii_without_escapes() {
    for (name, path) in common::fixtures() {
        let text = render(&common::load(&path), Mode::Plain, None);
        assert!(text.is_ascii() && !text.contains('\x1b'), "{name}:\n{text}");
    }
}

#[test]
fn colour_only_adds_escape_codes() {
    for (name, path) in common::fixtures() {
        let cpu = common::load(&path);
        let uncoloured = render(&cpu, Mode::Boxed, None);
        for theme in [Theme::Dark256, Theme::Light256, Theme::Ansi16] {
            let coloured = render(&cpu, Mode::Boxed, Some(theme));
            assert!(coloured.contains('\x1b'), "{name} {theme:?} has no colour");
            let stripped = common::strip_ansi(&coloured);
            assert_eq!(stripped, uncoloured, "{name} {theme:?}");
            assert!(
                stripped.lines().all(|l| UnicodeWidthStr::width(l) <= 80),
                "{name} {theme:?}"
            );
        }
    }
}
