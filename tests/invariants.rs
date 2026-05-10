//! Rules every fixture must satisfy. They need no expected output, so they also protect new
//! contributor snapshots that nobody has reviewed line by line.

mod common;

use std::collections::BTreeSet;

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
            for cache in &cluster.caches {
                if let (Some(s), Some(i), Some(t)) =
                    (&cache.shared_by, &cache.instances, &cluster.threads)
                {
                    assert_eq!(
                        s.value * i.value,
                        t.value,
                        "{name}: L{} of {}: shared_by × instances != threads",
                        cache.level,
                        cluster.label()
                    );
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
            let text = render(&cpu, mode, false);
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
        let text = render(&common::load(&path), Mode::Boxed, false);
        let widths: BTreeSet<usize> = text.lines().map(UnicodeWidthStr::width).collect();
        assert_eq!(widths.len(), 1, "{name}:\n{text}");
    }
}

#[test]
fn plain_output_is_ascii_without_escapes() {
    for (name, path) in common::fixtures() {
        let text = render(&common::load(&path), Mode::Plain, false);
        assert!(text.is_ascii() && !text.contains('\x1b'), "{name}:\n{text}");
    }
}
