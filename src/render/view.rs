//! Turns a [`Cpu`] into display-ready sections. Boxed and plain rendering both draw from this,
//! so they can never disagree about what is shown; they differ only in glyphs and framing.

use unicode_width::UnicodeWidthStr;

use crate::model::{Cache, CacheKind, Clocks, Cluster, Cpu, F};
use crate::units::Hertz;

/// Longest a value line may get before wrapping onto the next line.
pub const WRAP_WIDTH: usize = 56;

pub struct Glyphs {
    /// Between list items.
    pub sep: &'static str,
    /// Between items that themselves contain commas, such as CPU lists.
    pub set_sep: &'static str,
    /// In counts: `4 × Super`, `×2`.
    pub times: &'static str,
}

pub const UNICODE: Glyphs = Glyphs {
    sep: " · ",
    set_sep: " · ",
    times: "×",
};
pub const ASCII: Glyphs = Glyphs {
    sep: ", ",
    set_sep: " | ",
    times: "x",
};

#[derive(Debug, PartialEq)]
pub struct Section {
    pub title: &'static str,
    pub body: Body,
}

#[derive(Debug, PartialEq)]
pub enum Body {
    Pairs(Vec<Pair>),
    Grid(Grid),
}

/// A labelled value. Extra lines are wrapped continuations shown without a label.
#[derive(Debug, PartialEq)]
pub struct Pair {
    pub label: &'static str,
    pub lines: Vec<String>,
}

#[derive(Debug, PartialEq)]
pub struct Grid {
    pub corner: &'static str,
    pub columns: Vec<String>,
    pub rows: Vec<GridRow>,
}

#[derive(Debug, PartialEq)]
pub struct GridRow {
    pub label: String,
    /// One per column; empty when that column doesn't have this row. A spanning row has one cell.
    pub cells: Vec<String>,
    /// Drawn as one cell across every column, e.g. a cache shared by all core types.
    pub span: bool,
}

impl Grid {
    /// The corner label, then one label per column.
    pub fn header(&self) -> Vec<&str> {
        std::iter::once(self.corner)
            .chain(self.columns.iter().map(String::as_str))
            .collect()
    }

    /// The header and every non-spanning row, each starting with its label. Spanning rows are left
    /// out because they don't align to columns.
    pub fn table(&self) -> Vec<Vec<&str>> {
        let rows = self.rows.iter().filter(|r| !r.span).map(|r| {
            std::iter::once(r.label.as_str())
                .chain(r.cells.iter().map(String::as_str))
                .collect()
        });
        std::iter::once(self.header()).chain(rows).collect()
    }
}

/// Display width in terminal columns.
pub fn width(s: &str) -> usize {
    UnicodeWidthStr::width(s)
}

/// `s` padded with spaces to `w` columns.
pub fn pad(s: &str, w: usize) -> String {
    format!("{s}{}", " ".repeat(w.saturating_sub(width(s))))
}

pub fn column_widths(table: &[Vec<&str>]) -> Vec<usize> {
    let columns = table.iter().map(Vec::len).max().unwrap_or(0);
    (0..columns)
        .map(|i| {
            table
                .iter()
                .filter_map(|row| row.get(i))
                .map(|c| width(c))
                .max()
                .unwrap_or(0)
        })
        .collect()
}

/// Joins `items` with `sep`, starting a new line before any item that would pass `max` columns.
pub fn wrap(items: &[&str], sep: &str, max: usize) -> Vec<String> {
    let mut lines = Vec::new();
    let mut line = String::new();
    for item in items {
        if !line.is_empty() && width(&line) + width(sep) + width(item) > max {
            lines.push(std::mem::take(&mut line));
        }
        if !line.is_empty() {
            line.push_str(sep);
        }
        line.push_str(item);
    }
    if !line.is_empty() {
        lines.push(line);
    }
    lines
}

/// Formats sorted CPU numbers as a compact list: `0-3,8,10-11`.
pub fn format_cpu_list(cpus: &[u32]) -> String {
    let mut ranges = Vec::new();
    let mut iter = cpus.iter().copied().peekable();
    while let Some(start) = iter.next() {
        let mut end = start;
        while end
            .checked_add(1)
            .is_some_and(|next| iter.peek() == Some(&next))
        {
            end += 1;
            iter.next();
        }
        ranges.push(if start == end {
            start.to_string()
        } else {
            format!("{start}-{end}")
        });
    }
    ranges.join(",")
}

pub fn build(cpu: &Cpu, g: &Glyphs) -> Vec<Section> {
    [
        identity(cpu, g),
        topology(cpu, g),
        clocks(cpu),
        cache(cpu, g),
        features(cpu, g),
    ]
    .into_iter()
    .flatten()
    .collect()
}

/// A pairs section keeping only known rows; `None` when no row is known.
fn pairs(title: &'static str, rows: Vec<(&'static str, Option<Vec<String>>)>) -> Option<Section> {
    let rows: Vec<Pair> = rows
        .into_iter()
        .filter_map(|(label, lines)| {
            lines
                .filter(|l| !l.is_empty())
                .map(|lines| Pair { label, lines })
        })
        .collect();
    (!rows.is_empty()).then_some(Section {
        title,
        body: Body::Pairs(rows),
    })
}

fn one(s: Option<String>) -> Option<Vec<String>> {
    s.map(|s| vec![s])
}

fn val<T: ToString>(fact: &F<T>) -> Option<String> {
    fact.as_ref().map(|f| f.value.to_string())
}

fn identity(cpu: &Cpu, g: &Glyphs) -> Option<Section> {
    let id = &cpu.identity;
    let arch = val(&id.arch).map(|arch| match val(&id.isa_level) {
        Some(isa) => format!("{arch} ({isa})"),
        None => arch,
    });
    let hypervisor =
        val(&id.hypervisor).map(|h| format!("{h}{}topology as reported by guest", g.sep));
    pairs(
        "Identity",
        vec![
            ("Name", one(val(&id.name))),
            ("Vendor", one(val(&id.vendor))),
            ("Architecture", one(arch)),
            ("Microcode", one(val(&id.microcode))),
            ("Hypervisor", one(hypervisor)),
        ],
    )
}

fn topology(cpu: &Cpu, g: &Glyphs) -> Option<Section> {
    let t = &cpu.topology;
    let sockets = t
        .sockets
        .as_ref()
        .filter(|s| s.value > 1)
        .map(|s| s.value.to_string());
    let mut cores = Vec::new();
    if let Some(p) = &t.physical_cores {
        cores.push(format!("{} physical", p.value));
    }
    if let Some(l) = &t.logical_cpus {
        cores.push(format!("{} logical", l.value));
    }
    match t.smt_per_core.as_ref().map(|s| s.value) {
        Some(1) => cores.push("no SMT".to_string()),
        Some(n) => cores.push(format!("SMT {}{n}", g.times)),
        None => {}
    }
    let clusters = (cpu.clusters.len() > 1).then(|| {
        cpu.clusters
            .iter()
            .map(|c| match &c.cores {
                Some(n) => format!("{} {} {}", n.value, g.times, c.label()),
                None => c.label().to_string(),
            })
            .collect::<Vec<_>>()
            .join(g.sep)
    });
    let numa = (t.numa_nodes.len() > 1).then(|| {
        let lists: Vec<String> = t
            .numa_nodes
            .iter()
            .map(|n| format_cpu_list(&n.cpus))
            .collect();
        format!("{} nodes: {}", t.numa_nodes.len(), lists.join(g.set_sep))
    });
    pairs(
        "Topology",
        vec![
            ("Sockets", one(sockets)),
            (
                "Cores",
                (!cores.is_empty()).then(|| vec![cores.join(g.sep)]),
            ),
            ("Clusters", one(clusters)),
            ("NUMA", one(numa)),
        ],
    )
}

fn clock<'a>(clocks: &'a Clocks, label: &str) -> &'a F<Hertz> {
    match label {
        "Base" => &clocks.base,
        "Max" => &clocks.max,
        _ => &clocks.current,
    }
}

fn clocks(cpu: &Cpu) -> Option<Section> {
    let clusters: Vec<&Cluster> = cpu
        .clusters
        .iter()
        .filter(|c| !c.clock.is_empty())
        .collect();
    let rows: Vec<GridRow> = ["Base", "Max", "Current"]
        .into_iter()
        .filter_map(|label| {
            let cells: Vec<String> = clusters
                .iter()
                .map(|c| val(clock(&c.clock, label)).unwrap_or_default())
                .collect();
            cells.iter().any(|c| !c.is_empty()).then(|| GridRow {
                label: label.to_string(),
                cells,
                span: false,
            })
        })
        .collect();
    if rows.is_empty() {
        return None;
    }
    let columns = clusters.iter().map(|c| c.label().to_string()).collect();
    Some(Section {
        title: "Clocks",
        body: Body::Grid(Grid {
            corner: "",
            columns,
            rows,
        }),
    })
}

fn cache_label(level: u8, kind: CacheKind) -> String {
    format!("L{level}{}", kind.suffix())
}

fn cache(cpu: &Cpu, g: &Glyphs) -> Option<Section> {
    // A cache without a size (common in VMs) has nothing to show.
    let clusters: Vec<&Cluster> = cpu
        .clusters
        .iter()
        .filter(|c| c.caches.iter().any(|k| k.size.is_some()))
        .collect();
    if clusters.is_empty() {
        return None;
    }
    let mut keys: Vec<(u8, CacheKind)> = clusters
        .iter()
        .flat_map(|c| {
            c.caches
                .iter()
                .filter(|k| k.size.is_some())
                .map(|k| (k.level, k.kind))
        })
        .collect();
    keys.sort();
    keys.dedup();
    let mut rows: Vec<((u8, CacheKind), GridRow)> = keys
        .into_iter()
        .map(|(level, kind)| {
            let cells = clusters
                .iter()
                .map(|c| {
                    c.caches
                        .iter()
                        .find(|k| k.level == level && k.kind == kind)
                        .map(|k| cache_cell(k, c, g))
                        .unwrap_or_default()
                })
                .collect();
            (
                (level, kind),
                GridRow {
                    label: cache_label(level, kind),
                    cells,
                    span: false,
                },
            )
        })
        .collect();
    rows.extend(
        cpu.shared_caches
            .iter()
            .filter(|k| k.size.is_some())
            .map(|k| {
                let row = GridRow {
                    label: cache_label(k.level, k.kind),
                    cells: vec![shared_cell(k, cpu, g)],
                    span: true,
                };
                ((k.level, k.kind), row)
            }),
    );
    rows.sort_by_key(|(key, _)| *key);
    let columns = clusters.iter().map(|c| c.label().to_string()).collect();
    let rows = rows.into_iter().map(|(_, row)| row).collect();
    Some(Section {
        title: "Cache",
        body: Body::Grid(Grid {
            corner: "Level",
            columns,
            rows,
        }),
    })
}

/// `25 MiB shared by all 12 cores`, or `16 MiB / 8 CPUs` when shared by only some.
fn shared_cell(cache: &Cache, cpu: &Cpu, g: &Glyphs) -> String {
    let Some(size) = &cache.size else {
        return String::new();
    };
    let mut text = size.value.to_string();
    let logical = cpu.topology.logical_cpus.as_ref().map(|f| f.value);
    match cache.shared_by.as_ref().map(|f| f.value) {
        Some(n) if Some(n) == logical => match &cpu.topology.physical_cores {
            Some(p) => text.push_str(&format!(" shared by all {} cores", p.value)),
            None => text.push_str(" shared by all cores"),
        },
        Some(n) => text.push_str(&format!(" / {n} CPUs")),
        None => {}
    }
    if let Some(n) = cache.instances.as_ref().filter(|n| n.value > 1) {
        text.push_str(&format!(" {}{}", g.times, n.value));
    }
    text
}

/// `192 KiB / core`, `16 MiB / 4 cores`, or `16 MiB / 4 cores ×2` when the cluster has several.
fn cache_cell(cache: &Cache, cluster: &Cluster, g: &Glyphs) -> String {
    let Some(size) = &cache.size else {
        return String::new();
    };
    let mut text = size.value.to_string();
    if let Some(shared) = &cache.shared_by {
        let smt = match (&cluster.cores, &cluster.threads) {
            (Some(c), Some(t)) if c.value > 0 && t.value % c.value == 0 => t.value / c.value,
            _ => 1,
        };
        let cores = (shared.value / smt).max(1);
        if cores == 1 {
            text.push_str(" / core");
        } else {
            text.push_str(&format!(" / {cores} cores"));
            if let Some(n) = cache.instances.as_ref().filter(|n| n.value > 1) {
                text.push_str(&format!(" {}{}", g.times, n.value));
            }
        }
    }
    text
}

fn features(cpu: &Cpu, g: &Glyphs) -> Option<Section> {
    let rows = cpu
        .features
        .groups
        .iter()
        .map(|entry| {
            // Members of one family collapse into a single item at the first member's position.
            let mut items: Vec<(Option<&str>, Vec<&str>)> = Vec::new();
            for f in &entry.features {
                match f.family.as_deref() {
                    Some(family) => match items.iter_mut().find(|(fam, _)| *fam == Some(family)) {
                        Some((_, members)) => members.push(&f.name),
                        None => items.push((Some(family), vec![&f.name])),
                    },
                    None => items.push((None, vec![&f.name])),
                }
            }
            let names: Vec<String> = items
                .into_iter()
                .map(|(family, members)| match family {
                    Some(family) => format!("{family} ({})", members.join("/")),
                    None => members.join("/"),
                })
                .collect();
            let refs: Vec<&str> = names.iter().map(String::as_str).collect();
            (entry.group.label(), Some(wrap(&refs, g.sep, WRAP_WIDTH)))
        })
        .collect();
    pairs("Features", rows)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::Fact;
    use crate::test_support::fixture;

    fn titles(sections: &[Section]) -> Vec<&str> {
        sections.iter().map(|s| s.title).collect()
    }

    fn grid(sections: &[Section]) -> &Grid {
        sections
            .iter()
            .find_map(|s| match &s.body {
                Body::Grid(g) => Some(g),
                Body::Pairs(_) => None,
            })
            .expect("a grid section")
    }

    fn row<'a>(sections: &'a [Section], label: &str) -> &'a Pair {
        sections
            .iter()
            .find_map(|s| match &s.body {
                Body::Pairs(pairs) => pairs.iter().find(|p| p.label == label),
                Body::Grid(_) => None,
            })
            .unwrap_or_else(|| panic!("no row {label}"))
    }

    #[test]
    fn apple_m5_sections() {
        let s = build(&fixture("apple-m5"), &UNICODE);
        assert_eq!(titles(&s), ["Identity", "Topology", "Cache", "Features"]);
        assert_eq!(row(&s, "Clusters").lines, ["4 × Super · 6 × Efficiency"]);
        assert_eq!(
            row(&s, "Cores").lines,
            ["10 physical · 10 logical · no SMT"]
        );
    }

    #[test]
    fn cache_grid_has_a_column_per_cluster() {
        let s = build(&fixture("apple-m5"), &UNICODE);
        let g = grid(&s);
        assert_eq!(g.columns, ["Super", "Efficiency"]);
        let labels: Vec<&str> = g.rows.iter().map(|r| r.label.as_str()).collect();
        assert_eq!(labels, ["L1i", "L1d", "L2"]);
        assert_eq!(g.rows[0].cells, ["192 KiB / core", "128 KiB / core"]);
        assert_eq!(g.rows[2].cells, ["16 MiB / 4 cores", "6 MiB / 6 cores"]);
    }

    #[test]
    fn repeated_shared_caches_show_their_count() {
        let s = build(&fixture("apple-m2-pro"), &UNICODE);
        assert_eq!(
            grid(&s).rows[2].cells,
            ["16 MiB / 4 cores ×2", "4 MiB / 4 cores"]
        );
    }

    #[test]
    fn ascii_glyphs() {
        let s = build(&fixture("apple-m5"), &ASCII);
        assert_eq!(row(&s, "Clusters").lines, ["4 x Super, 6 x Efficiency"]);
    }

    #[test]
    fn unknown_sections_and_rows_are_hidden() {
        let s = build(&fixture("sparse-mac"), &UNICODE);
        assert_eq!(titles(&s), ["Identity", "Topology"]);
        match &s[1].body {
            Body::Pairs(pairs) => {
                assert_eq!(pairs.iter().map(|p| p.label).collect::<Vec<_>>(), ["Cores"])
            }
            Body::Grid(_) => panic!("topology is pairs"),
        }
    }

    #[test]
    fn smt_is_shown_as_a_multiplier() {
        let mut cpu = Cpu::default();
        cpu.topology.physical_cores = Some(Fact::derived(8));
        cpu.topology.logical_cpus = Some(Fact::derived(16));
        cpu.topology.smt_per_core = Some(Fact::derived(2));
        assert_eq!(
            row(&build(&cpu, &UNICODE), "Cores").lines,
            ["8 physical · 16 logical · SMT ×2"]
        );
    }

    #[test]
    fn wrap_breaks_between_items() {
        assert_eq!(wrap(&["a", "b", "c"], " · ", 5), ["a · b", "c"]);
        assert_eq!(wrap(&["abcdefgh"], ", ", 3), ["abcdefgh"]);
        assert!(wrap(&[], ", ", 10).is_empty());
    }

    #[test]
    fn feature_families_collapse_into_one_item() {
        use crate::model::{Feature, FeatureGroup, FeatureGroupEntry};
        let feature = |raw: &str, name: &str, family: Option<&str>| Feature {
            raw: raw.into(),
            name: name.into(),
            family: family.map(str::to_string),
        };
        let mut cpu = Cpu::default();
        cpu.features.groups = vec![FeatureGroupEntry {
            group: FeatureGroup::Simd,
            features: vec![
                feature("avx2", "AVX2", None),
                feature("avx512f", "F", Some("AVX-512")),
                feature("avx512bw", "BW", Some("AVX-512")),
                feature("amx_tile", "TILE", Some("AMX")),
            ],
        }];
        assert_eq!(
            row(&build(&cpu, &UNICODE), "SIMD").lines,
            ["AVX2 · AVX-512 (F/BW) · AMX (TILE)"]
        );
    }

    fn grid_titled<'a>(sections: &'a [Section], title: &str) -> &'a Grid {
        sections
            .iter()
            .find_map(|s| match &s.body {
                Body::Grid(g) if s.title == title => Some(g),
                _ => None,
            })
            .unwrap_or_else(|| panic!("no {title} grid"))
    }

    #[test]
    fn linux_sections_in_order() {
        let s = build(&fixture("linux-x86-intel-hybrid"), &UNICODE);
        assert_eq!(
            titles(&s),
            ["Identity", "Topology", "Clocks", "Cache", "Features"]
        );
    }

    #[test]
    fn clocks_grid_per_core_type() {
        let s = build(&fixture("linux-x86-intel-hybrid"), &UNICODE);
        let g = grid_titled(&s, "Clocks");
        assert_eq!(g.columns, ["Performance", "Efficiency"]);
        let rows: Vec<(&str, Vec<&str>)> = g
            .rows
            .iter()
            .map(|r| {
                (
                    r.label.as_str(),
                    r.cells.iter().map(String::as_str).collect(),
                )
            })
            .collect();
        assert_eq!(
            rows,
            [
                ("Base", vec!["3.60 GHz", "2.70 GHz"]),
                ("Max", vec!["5.00 GHz", "3.80 GHz"])
            ]
        );
    }

    #[test]
    fn a_cache_shared_by_every_core_spans_all_columns() {
        let s = build(&fixture("linux-x86-intel-hybrid"), &UNICODE);
        let g = grid_titled(&s, "Cache");
        let l2 = g.rows.iter().find(|r| r.label == "L2").unwrap();
        assert_eq!(l2.cells, ["1280 KiB / core", "2 MiB / 4 cores"]);
        let l3 = g.rows.last().unwrap();
        assert_eq!(
            (l3.label.as_str(), l3.span, l3.cells.as_slice()),
            (
                "L3",
                true,
                &["25 MiB shared by all 12 cores".to_string()][..]
            )
        );
    }

    #[test]
    fn hypervisor_and_numa_rows() {
        let s = build(&fixture("linux-x86-amd-gce"), &UNICODE);
        assert_eq!(
            row(&s, "Hypervisor").lines,
            ["Google Compute Engine · topology as reported by guest"]
        );
        assert_eq!(row(&s, "NUMA").lines, ["2 nodes: 0-3,8-11 · 4-7,12-15"]);
        assert_eq!(
            grid_titled(&s, "Cache").rows.last().unwrap().cells,
            ["16 MiB / 4 cores ×2"]
        );
        let ascii = build(&fixture("linux-x86-amd-gce"), &ASCII);
        assert_eq!(row(&ascii, "NUMA").lines, ["2 nodes: 0-3,8-11 | 4-7,12-15"]);
    }

    #[test]
    fn cpu_lists_compress_ranges() {
        assert_eq!(format_cpu_list(&[0, 1, 2, 3, 8, 10, 11]), "0-3,8,10-11");
        assert_eq!(format_cpu_list(&[5]), "5");
        assert_eq!(format_cpu_list(&[]), "");
    }
}
