//! Turns a [`Cpu`] into display-ready sections. Boxed and plain rendering both draw from this,
//! so they can never disagree about what is shown; they differ only in glyphs and framing.
//!
//! Every value is a [`Line`]: its text plus the [`Role`] of each styled piece, so the boxed
//! renderer can colour a number, a unit or a core type without re-parsing text. Everything else
//! (widths, plain output, tests) uses the text alone.

use std::fmt;
use std::ops::{Deref, Range};

use unicode_width::UnicodeWidthStr;

use crate::model::{Cache, CacheKind, Clocks, Cluster, CoreKind, Cpu, F, FeatureGroup, Origin};
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
    /// Marks a value from a built-in table.
    pub dagger: &'static str,
}

pub const UNICODE: Glyphs = Glyphs {
    sep: " · ",
    set_sep: " · ",
    times: "×",
    dagger: "†",
};
pub const ASCII: Glyphs = Glyphs {
    sep: ", ",
    set_sep: " | ",
    times: "x",
    dagger: "*",
};

/// What a piece of a value is, for colouring. Text outside any styled piece is [`Role::Plain`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Role {
    Plain,
    /// Counts, sizes and frequencies: `16`, `4.46`.
    Number,
    /// `KiB`, `MiB`, `GHz`.
    Unit,
    /// `·`, `/`, `×`, `+`, `,`, `|`.
    Separator,
    /// A core type, wherever one is named (including OS names such as "Super").
    Kind(CoreKind),
    /// One feature name, e.g. `SSE4.2` or `AVX-512 (F/BW)`; never split at digits.
    Feature(FeatureGroup),
    /// A cache level row label: 1, 2 or 3.
    Level(u8),
    /// The CPU name, tinted by vendor.
    Vendor(VendorTint),
    /// A caveat about the other values: `topology as reported by guest`, Rosetta.
    Note,
    /// `†` (`*` in ASCII) on a value from a built-in table.
    Marker,
}

/// Which vendor colour the CPU name gets.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum VendorTint {
    Intel,
    Amd,
    Apple,
    Other,
}

impl VendorTint {
    /// From the model's vendor, never from the name text.
    fn of(vendor: &F<String>) -> VendorTint {
        match vendor.as_ref().map(|f| f.value.as_str()) {
            Some("Intel") => VendorTint::Intel,
            Some("AMD") => VendorTint::Amd,
            Some("Apple") => VendorTint::Apple,
            _ => VendorTint::Other,
        }
    }
}

/// Display text plus the role of each styled piece.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Line {
    text: String,
    spans: Vec<(Range<usize>, Role)>,
}

impl Line {
    pub fn new() -> Line {
        Line::default()
    }

    pub fn plain(text: &str) -> Line {
        Line::styled(text, Role::Plain)
    }

    pub fn styled(text: &str, role: Role) -> Line {
        Line::new().with(text, role)
    }

    pub fn with(mut self, text: &str, role: Role) -> Line {
        self.push(text, role);
        self
    }

    pub fn with_line(mut self, other: &Line) -> Line {
        self.append(other);
        self
    }

    pub fn push(&mut self, text: &str, role: Role) {
        let start = self.text.len();
        self.text.push_str(text);
        if role != Role::Plain && !text.is_empty() {
            self.spans.push((start..self.text.len(), role));
        }
    }

    pub fn append(&mut self, other: &Line) {
        let offset = self.text.len();
        self.text.push_str(&other.text);
        self.spans.extend(
            other
                .spans
                .iter()
                .map(|(r, role)| (r.start + offset..r.end + offset, *role)),
        );
    }

    pub fn as_str(&self) -> &str {
        &self.text
    }

    /// Every piece of text in order with its role; text between styled pieces is `Plain`.
    pub fn pieces(&self) -> Vec<(&str, Role)> {
        let mut out = Vec::new();
        let mut at = 0;
        for (range, role) in &self.spans {
            if range.start > at {
                out.push((&self.text[at..range.start], Role::Plain));
            }
            out.push((&self.text[range.clone()], *role));
            at = range.end;
        }
        if at < self.text.len() {
            out.push((&self.text[at..], Role::Plain));
        }
        out
    }

    pub fn join(items: &[Line], sep: &Line) -> Line {
        let mut line = Line::new();
        for (i, item) in items.iter().enumerate() {
            if i > 0 {
                line.append(sep);
            }
            line.append(item);
        }
        line
    }
}

impl Deref for Line {
    type Target = str;
    fn deref(&self) -> &str {
        &self.text
    }
}

impl fmt::Display for Line {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.text)
    }
}

impl PartialEq<&str> for Line {
    fn eq(&self, other: &&str) -> bool {
        self.text == *other
    }
}

impl PartialEq<String> for Line {
    fn eq(&self, other: &String) -> bool {
        &self.text == other
    }
}

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
    pub label: Line,
    pub lines: Vec<Line>,
}

#[derive(Debug, PartialEq)]
pub struct Grid {
    pub corner: &'static str,
    pub columns: Vec<Line>,
    pub rows: Vec<GridRow>,
}

#[derive(Debug, PartialEq)]
pub struct GridRow {
    pub label: Line,
    /// One per column; empty when that column doesn't have this row. A spanning row has one cell.
    pub cells: Vec<Line>,
    /// Drawn as one cell across every column, e.g. a cache shared by all core types.
    pub span: bool,
}

impl Grid {
    /// The corner label, then one label per column.
    pub fn header(&self) -> Vec<&str> {
        std::iter::once(self.corner)
            .chain(self.columns.iter().map(Line::as_str))
            .collect()
    }

    /// The header and every non-spanning row, each starting with its label. Spanning rows are left
    /// out because they don't align to columns.
    pub fn table(&self) -> Vec<Vec<&str>> {
        let rows = self.rows.iter().filter(|r| !r.span).map(|r| {
            std::iter::once(r.label.as_str())
                .chain(r.cells.iter().map(Line::as_str))
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
pub fn wrap(items: &[Line], sep: &Line, max: usize) -> Vec<Line> {
    let mut lines = Vec::new();
    let mut line = Line::new();
    for item in items {
        if !line.is_empty() && width(&line) + width(sep) + width(item) > max {
            lines.push(std::mem::take(&mut line));
        }
        if !line.is_empty() {
            line.append(sep);
        }
        line.append(item);
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

/// Glyph text as a line: its non-space part gets `role` (` · ` is a space, `·`, a space).
fn glyph(text: &str, role: Role) -> Line {
    let core = text.trim();
    let start = text.len() - text.trim_start().len();
    Line::plain(&text[..start])
        .with(core, role)
        .with(&text[start + core.len()..], Role::Plain)
}

fn sep(text: &str) -> Line {
    glyph(text, Role::Separator)
}

fn number(n: impl ToString) -> Line {
    Line::styled(&n.to_string(), Role::Number)
}

/// `4.46 GHz` or `16 MiB`: the number, then its unit.
fn quantity(text: &str) -> Line {
    match text.rsplit_once(' ') {
        Some((n, unit)) => Line::styled(n, Role::Number)
            .with(" ", Role::Plain)
            .with(unit, Role::Unit),
        None => Line::styled(text, Role::Number),
    }
}

/// A value's text in `role`, marked when it came from a built-in table rather than the machine.
fn shown<T: ToString>(fact: &F<T>, g: &Glyphs, role: Role) -> Option<Line> {
    let f = fact.as_ref()?;
    let line = Line::styled(&f.value.to_string(), role);
    Some(match f.origin {
        Origin::Database(_) => line.with(" ", Role::Plain).with(g.dagger, Role::Marker),
        _ => line,
    })
}

/// A cluster's heading in its core type's role, marked like [`shown`] when its name came from a
/// built-in table.
fn cluster_label(cluster: &Cluster, g: &Glyphs) -> Line {
    let role = Role::Kind(cluster.kind);
    shown(&cluster.name, g, role).unwrap_or_else(|| Line::styled(cluster.kind.label(), role))
}

/// `† from a built-in table: arm-midr@2026-09`, when any shown value came from one.
pub fn footnote(cpu: &Cpu, g: &Glyphs) -> Option<Line> {
    let mut tables: Vec<&str> = [&cpu.identity.name, &cpu.identity.vendor]
        .into_iter()
        .chain(cpu.clusters.iter().map(|c| &c.name))
        .filter_map(|f| match f.as_ref()?.origin {
            Origin::Database(table) => Some(table),
            _ => None,
        })
        .collect();
    tables.sort_unstable();
    tables.dedup();
    (!tables.is_empty()).then(|| {
        Line::styled(g.dagger, Role::Marker).with(
            &format!(" from a built-in table: {}", tables.join(", ")),
            Role::Plain,
        )
    })
}

/// Column widths for a grid: the first column fits the title in its top border, and the last
/// column grows until every spanning row's text fits.
pub fn grid_widths(title: &str, grid: &Grid) -> Vec<usize> {
    let mut widths = column_widths(&grid.table());
    widths[0] = widths[0].max(width(title) + 1);
    for row in grid.rows.iter().filter(|r| r.span) {
        widths[0] = widths[0].max(width(&row.label));
        let text = row.cells.first().map_or(0, |c| width(c));
        let available = span_width(&widths);
        if text > available {
            let last = widths.len() - 1;
            widths[last] += text - available;
        }
    }
    widths
}

/// Text width of a cell spanning every column after the first, including the separators it covers.
pub fn span_width(widths: &[usize]) -> usize {
    widths[1..].iter().sum::<usize>() + 3 * (widths.len() - 2)
}

pub fn grid_inner(widths: &[usize]) -> usize {
    widths.iter().map(|w| w + 2).sum::<usize>() + widths.len() - 1
}

/// Widest box content that still fits an 80-column terminal (80 minus the two borders).
pub const MAX_INNER: usize = 78;

/// A grid too wide for 80 columns becomes one row per cluster: `Label  L1d 48 KiB / core · ...`.
fn fit(section: Section, g: &Glyphs) -> Section {
    let Body::Grid(grid) = &section.body else {
        return section;
    };
    if grid_inner(&grid_widths(section.title, grid)) <= MAX_INNER {
        return section;
    }
    let mut pairs: Vec<Pair> = grid
        .columns
        .iter()
        .enumerate()
        .map(|(i, column)| {
            let items: Vec<Line> = grid
                .rows
                .iter()
                .filter(|r| !r.span)
                .filter_map(|r| {
                    r.cells
                        .get(i)
                        .filter(|c| !c.is_empty())
                        .map(|c| r.label.clone().with(" ", Role::Plain).with_line(c))
                })
                .collect();
            Pair {
                label: column.clone(),
                lines: wrap(&items, &sep(g.sep), WRAP_WIDTH),
            }
        })
        .filter(|p| !p.lines.is_empty())
        .collect();
    let shared: Vec<Line> = grid
        .rows
        .iter()
        .filter(|r| r.span)
        .map(|r| {
            r.label
                .clone()
                .with(" ", Role::Plain)
                .with_line(&Line::join(&r.cells, &Line::new()))
        })
        .collect();
    if !shared.is_empty() {
        pairs.push(Pair {
            label: Line::plain("Shared"),
            lines: wrap(&shared, &sep(g.sep), WRAP_WIDTH),
        });
    }
    Section {
        title: section.title,
        body: Body::Pairs(pairs),
    }
}

pub fn build(cpu: &Cpu, g: &Glyphs) -> Vec<Section> {
    [
        identity(cpu, g),
        topology(cpu, g),
        clocks(cpu, g).map(|s| fit(s, g)),
        cache(cpu, g).map(|s| fit(s, g)),
        features(cpu, g),
    ]
    .into_iter()
    .flatten()
    .collect()
}

/// A pairs section keeping only known rows; `None` when no row is known.
fn pairs(title: &'static str, rows: Vec<(&'static str, Option<Vec<Line>>)>) -> Option<Section> {
    let rows: Vec<Pair> = rows
        .into_iter()
        .filter_map(|(label, lines)| {
            lines.filter(|l| !l.is_empty()).map(|lines| Pair {
                label: Line::plain(label),
                lines,
            })
        })
        .collect();
    (!rows.is_empty()).then_some(Section {
        title,
        body: Body::Pairs(rows),
    })
}

fn one(line: Option<Line>) -> Option<Vec<Line>> {
    line.map(|l| vec![l])
}

fn val<T: ToString>(fact: &F<T>) -> Option<String> {
    fact.as_ref().map(|f| f.value.to_string())
}

fn identity(cpu: &Cpu, g: &Glyphs) -> Option<Section> {
    let id = &cpu.identity;
    let translated = id.translated.as_ref().is_some_and(|t| t.value);
    let arch = val(&id.arch).map(|arch| {
        let mut line = Line::plain(&arch);
        if let Some(isa) = val(&id.isa_level) {
            line.push(&format!(" ({isa})"), Role::Plain);
        }
        if translated {
            line.append(&sep(g.sep));
            line.push("x86_64 binary running under Rosetta 2", Role::Note);
        }
        line
    });
    let hypervisor = val(&id.hypervisor).map(|h| {
        wrap(
            &[
                Line::plain(&h),
                Line::styled("topology as reported by guest", Role::Note),
            ],
            &sep(g.sep),
            WRAP_WIDTH,
        )
    });
    pairs(
        "Identity",
        vec![
            (
                "Name",
                one(shown(&id.name, g, Role::Vendor(VendorTint::of(&id.vendor)))),
            ),
            ("Vendor", one(shown(&id.vendor, g, Role::Plain))),
            ("Architecture", one(arch)),
            (
                "Microcode",
                one(val(&id.microcode).map(|m| Line::plain(&m))),
            ),
            ("Hypervisor", hypervisor),
        ],
    )
}

fn topology(cpu: &Cpu, g: &Glyphs) -> Option<Section> {
    let t = &cpu.topology;
    let sockets = t
        .sockets
        .as_ref()
        .filter(|s| s.value > 1)
        .map(|s| number(s.value));
    let mut cores = Vec::new();
    if let Some(p) = &t.physical_cores {
        cores.push(number(p.value).with(" physical", Role::Plain));
    }
    if let Some(l) = &t.logical_cpus {
        cores.push(number(l.value).with(" logical", Role::Plain));
    }
    match t.smt_per_core.as_ref().map(|s| s.value) {
        Some(1) => cores.push(Line::plain("no SMT")),
        Some(n) => cores.push(
            Line::plain("SMT ")
                .with(g.times, Role::Separator)
                .with_line(&number(n)),
        ),
        None => {}
    }
    let clusters = (cpu.clusters.len() > 1).then(|| {
        let items: Vec<Line> = cpu
            .clusters
            .iter()
            .map(|c| match &c.cores {
                Some(n) => number(n.value)
                    .with(" ", Role::Plain)
                    .with(g.times, Role::Separator)
                    .with(" ", Role::Plain)
                    .with_line(&cluster_label(c, g)),
                None => cluster_label(c, g),
            })
            .collect();
        wrap(&items, &sep(g.sep), WRAP_WIDTH)
    });
    let numa = (t.numa_nodes.len() > 1).then(|| {
        let lists: Vec<Line> = t
            .numa_nodes
            .iter()
            .map(|n| Line::plain(&format_cpu_list(&n.cpus)))
            .collect();
        let prefix = number(t.numa_nodes.len()).with(" nodes: ", Role::Plain);
        let mut lines = wrap(&lists, &sep(g.set_sep), WRAP_WIDTH - width(&prefix));
        if let Some(first) = lines.first_mut() {
            *first = prefix.with_line(first);
        }
        lines
    });
    pairs(
        "Topology",
        vec![
            ("Sockets", one(sockets)),
            (
                "Cores",
                (!cores.is_empty()).then(|| vec![Line::join(&cores, &sep(g.sep))]),
            ),
            ("Clusters", clusters),
            ("NUMA", numa),
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

fn clocks(cpu: &Cpu, g: &Glyphs) -> Option<Section> {
    let clusters: Vec<&Cluster> = cpu
        .clusters
        .iter()
        .filter(|c| !c.clock.is_empty())
        .collect();
    let rows: Vec<GridRow> = ["Base", "Max", "Current"]
        .into_iter()
        .filter_map(|label| {
            let cells: Vec<Line> = clusters
                .iter()
                .map(|c| {
                    val(clock(&c.clock, label))
                        .map(|v| quantity(&v))
                        .unwrap_or_default()
                })
                .collect();
            cells.iter().any(|c| !c.is_empty()).then(|| GridRow {
                label: Line::plain(label),
                cells,
                span: false,
            })
        })
        .collect();
    if rows.is_empty() {
        return None;
    }
    let columns = clusters.iter().map(|c| cluster_label(c, g)).collect();
    Some(Section {
        title: "Clocks",
        body: Body::Grid(Grid {
            corner: "",
            columns,
            rows,
        }),
    })
}

fn cache_label(level: u8, kind: CacheKind) -> Line {
    Line::styled(&format!("L{level}{}", kind.suffix()), Role::Level(level))
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
                    let parts: Vec<Line> = c
                        .caches
                        .iter()
                        .filter(|k| k.level == level && k.kind == kind && k.size.is_some())
                        .map(|k| cache_cell(k, g))
                        .collect();
                    Line::join(&parts, &sep(" + "))
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
    let columns = clusters.iter().map(|c| cluster_label(c, g)).collect();
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

/// ` ×2` when a cache has several instances.
fn instances(line: &mut Line, cache: &Cache, g: &Glyphs) {
    if let Some(n) = cache.instances.as_ref().filter(|n| n.value > 1) {
        line.push(" ", Role::Plain);
        line.push(g.times, Role::Separator);
        line.append(&number(n.value));
    }
}

/// `25 MiB shared by all 12 cores`, or `16 MiB / 8 CPUs` when shared by only some.
fn shared_cell(cache: &Cache, cpu: &Cpu, g: &Glyphs) -> Line {
    let Some(size) = &cache.size else {
        return Line::new();
    };
    let mut line = quantity(&size.value.to_string());
    let logical = cpu.topology.logical_cpus.as_ref().map(|f| f.value);
    match cache.shared_by.as_ref().map(|f| f.value) {
        Some(n) if Some(n) == logical => match &cpu.topology.physical_cores {
            Some(p) => {
                line.push(" shared by all ", Role::Plain);
                line.append(&number(p.value));
                line.push(" cores", Role::Plain);
            }
            None => line.push(" shared by all cores", Role::Plain),
        },
        Some(n) => {
            line.append(&sep(" / "));
            match &cache.cores {
                Some(c) => {
                    line.append(&number(c.value));
                    line.push(" cores", Role::Plain);
                }
                None => {
                    line.append(&number(n));
                    line.push(" CPUs", Role::Plain);
                }
            }
        }
        None => {}
    }
    instances(&mut line, cache, g);
    line
}

/// `192 KiB / core`, `16 MiB / 4 cores ×2`, or `16 MiB / 8 CPUs` when the core count is unknown.
fn cache_cell(cache: &Cache, g: &Glyphs) -> Line {
    let Some(size) = &cache.size else {
        return Line::new();
    };
    let mut line = quantity(&size.value.to_string());
    let (n, unit) = match (&cache.cores, &cache.shared_by) {
        (Some(cores), _) => (cores.value, "core"),
        (None, Some(cpus)) => (cpus.value, "CPU"),
        (None, None) => return line,
    };
    line.append(&sep(" / "));
    if n == 1 {
        line.push(unit, Role::Plain);
    } else {
        line.append(&number(n));
        line.push(&format!(" {unit}s"), Role::Plain);
        instances(&mut line, cache, g);
    }
    line
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
            let names: Vec<Line> = items
                .into_iter()
                .map(|(family, members)| {
                    let name = match family {
                        Some(family) => format!("{family} ({})", members.join("/")),
                        None => members.join("/"),
                    };
                    Line::styled(&name, Role::Feature(entry.group))
                })
                .collect();
            (
                entry.group.label(),
                Some(wrap(&names, &sep(g.sep), WRAP_WIDTH)),
            )
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
        assert_eq!(
            titles(&s),
            ["Identity", "Topology", "Clocks", "Cache", "Features"]
        );
        assert_eq!(row(&s, "Clusters").lines, ["4 × Super · 6 × Efficiency"]);
        assert_eq!(
            row(&s, "Cores").lines,
            ["10 physical · 10 logical · no SMT"]
        );
    }

    #[test]
    fn cache_grid_has_a_column_per_cluster() {
        let s = build(&fixture("apple-m5"), &UNICODE);
        let g = grid_titled(&s, "Cache");
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
                assert_eq!(
                    pairs.iter().map(|p| p.label.as_str()).collect::<Vec<_>>(),
                    ["Cores"]
                )
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
        let items = |xs: &[&str]| xs.iter().map(|x| Line::plain(x)).collect::<Vec<_>>();
        let dot = Line::plain(" · ");
        assert_eq!(wrap(&items(&["a", "b", "c"]), &dot, 5), ["a · b", "c"]);
        assert_eq!(
            wrap(&items(&["abcdefgh"]), &Line::plain(", "), 3),
            ["abcdefgh"]
        );
        assert!(wrap(&[], &dot, 10).is_empty());
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
            .map(|r| (r.label.as_str(), r.cells.iter().map(Line::as_str).collect()))
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
        assert_eq!((l3.label.as_str(), l3.span), ("L3", true));
        assert_eq!(l3.cells, ["25 MiB shared by all 12 cores"]);
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

    fn one_cluster_with(caches: Vec<Cache>) -> Cpu {
        Cpu {
            clusters: vec![Cluster {
                caches,
                ..Cluster::default()
            }],
            ..Cpu::default()
        }
    }

    fn l3(size_mib: u64, shared_by: u32, cores: Option<u32>, instances: u32) -> Cache {
        Cache {
            level: 3,
            kind: CacheKind::Unified,
            size: Some(Fact::derived(crate::units::Bytes(size_mib << 20))),
            shared_by: Some(Fact::derived(shared_by)),
            cores: cores.map(Fact::derived),
            instances: Some(Fact::derived(instances)),
        }
    }

    #[test]
    fn differing_cache_instances_are_joined() {
        let cpu = one_cluster_with(vec![l3(96, 16, Some(8), 1), l3(32, 16, Some(8), 1)]);
        let s = build(&cpu, &UNICODE);
        assert_eq!(
            grid(&s).rows[0].cells,
            ["96 MiB / 8 cores + 32 MiB / 8 cores"]
        );
    }

    #[test]
    fn caches_without_a_core_count_say_cpus() {
        let cpu = one_cluster_with(vec![l3(16, 8, None, 2)]);
        assert_eq!(
            grid(&build(&cpu, &UNICODE)).rows[0].cells,
            ["16 MiB / 8 CPUs ×2"]
        );
    }

    #[test]
    fn long_hypervisor_and_numa_rows_wrap() {
        let mut cpu = Cpu::default();
        cpu.identity.hypervisor = Some(Fact::derived(
            "Some Very Long Virtual Platform Name Here".to_string(),
        ));
        cpu.topology.numa_nodes = (0..8)
            .map(|id| crate::model::NumaNode {
                id,
                cpus: (id * 32..id * 32 + 16)
                    .chain(id * 32 + 256..id * 32 + 272)
                    .collect(),
            })
            .collect();
        let s = build(&cpu, &UNICODE);
        for label in ["Hypervisor", "NUMA"] {
            let lines = &row(&s, label).lines;
            assert!(lines.len() > 1, "{label} should wrap: {lines:?}");
            assert!(
                lines.iter().all(|l| width(l) <= WRAP_WIDTH),
                "{label}: {lines:?}"
            );
        }
        assert!(row(&s, "NUMA").lines[0].starts_with("8 nodes: 0-15,256-271 · "));
    }

    #[test]
    fn rosetta_is_named_in_the_architecture_row() {
        let s = build(&fixture("apple-m5-rosetta"), &UNICODE);
        assert_eq!(
            row(&s, "Architecture").lines,
            ["arm64 · x86_64 binary running under Rosetta 2"]
        );
    }

    #[test]
    fn table_values_are_marked_and_footnoted() {
        let cpu = fixture("linux-arm64-oci-a1");
        let s = build(&cpu, &UNICODE);
        assert_eq!(row(&s, "Name").lines, ["Neoverse-N1 †"]);
        assert_eq!(row(&s, "Vendor").lines, ["ARM †"]);
        assert_eq!(
            footnote(&cpu, &UNICODE).as_deref(),
            Some("† from a built-in table: arm-midr@2026-09")
        );
        assert_eq!(
            footnote(&cpu, &ASCII).as_deref(),
            Some("* from a built-in table: arm-midr@2026-09")
        );
        assert_eq!(footnote(&fixture("apple-m5"), &UNICODE), None);
    }

    #[test]
    fn wide_grids_become_rows() {
        let caches = || {
            vec![crate::model::Cache {
                level: 2,
                kind: CacheKind::Unified,
                size: Some(Fact::derived(crate::units::Bytes(2 << 20))),
                shared_by: Some(Fact::derived(4)),
                cores: Some(Fact::derived(4)),
                instances: Some(Fact::derived(1)),
            }]
        };
        let cluster = |name: &str| Cluster {
            name: Some(Fact::derived(name.to_string())),
            caches: caches(),
            ..Cluster::default()
        };
        let cpu = Cpu {
            clusters: [
                "Prime",
                "Performance-high",
                "Performance-mid",
                "Efficiency-big",
                "Efficiency-little",
            ]
            .map(cluster)
            .to_vec(),
            ..Cpu::default()
        };
        let s = build(&cpu, &UNICODE);
        let cache = s.iter().find(|s| s.title == "Cache").unwrap();
        let Body::Pairs(pairs) = &cache.body else {
            panic!("expected rows per cluster")
        };
        let labels: Vec<&str> = pairs.iter().map(|p| p.label.as_str()).collect();
        assert_eq!(
            labels,
            [
                "Prime",
                "Performance-high",
                "Performance-mid",
                "Efficiency-big",
                "Efficiency-little"
            ]
        );
        assert_eq!(pairs[0].lines, ["L2 2 MiB / 4 cores"]);
        assert_eq!(
            pairs[0].label.pieces(),
            [("Prime", Role::Kind(crate::model::CoreKind::Uniform))]
        );
        assert_eq!(pairs[0].lines[0].pieces()[0], ("L2", Role::Level(2)));
        let text = super::super::boxed::render(&s, None);
        assert!(text.lines().all(|l| width(l) <= 80), "{text}");
    }

    #[test]
    fn values_carry_roles() {
        use crate::model::{CoreKind, FeatureGroup};
        let s = build(&fixture("linux-x86-intel-hybrid"), &UNICODE);
        assert_eq!(
            row(&s, "Name").lines[0].pieces(),
            [(
                "12th Gen Intel(R) Core(TM) i7-12700K",
                Role::Vendor(VendorTint::Intel)
            )]
        );
        assert_eq!(
            row(&s, "Clusters").lines[0].pieces()[..5],
            [
                ("8", Role::Number),
                (" ", Role::Plain),
                ("×", Role::Separator),
                (" ", Role::Plain),
                ("Performance", Role::Kind(CoreKind::Performance)),
            ]
        );
        let cache = grid_titled(&s, "Cache");
        assert_eq!(
            cache.columns[1].pieces(),
            [("Efficiency", Role::Kind(CoreKind::Efficiency))]
        );
        let l2 = cache.rows.iter().find(|r| r.label == "L2").unwrap();
        assert_eq!(l2.label.pieces(), [("L2", Role::Level(2))]);
        assert_eq!(
            l2.cells[1].pieces(),
            [
                ("2", Role::Number),
                (" ", Role::Plain),
                ("MiB", Role::Unit),
                (" ", Role::Plain),
                ("/", Role::Separator),
                (" ", Role::Plain),
                ("4", Role::Number),
                (" cores", Role::Plain),
            ]
        );
        assert_eq!(
            cache.rows.last().unwrap().cells[0].pieces(),
            [
                ("25", Role::Number),
                (" ", Role::Plain),
                ("MiB", Role::Unit),
                (" shared by all ", Role::Plain),
                ("12", Role::Number),
                (" cores", Role::Plain),
            ]
        );
        assert_eq!(
            grid_titled(&s, "Clocks").rows[0].cells[0].pieces(),
            [
                ("3.60", Role::Number),
                (" ", Role::Plain),
                ("GHz", Role::Unit)
            ]
        );
        assert_eq!(
            row(&s, "SIMD").lines[0].pieces()[..3],
            [
                ("SSE4.2", Role::Feature(FeatureGroup::Simd)),
                (" ", Role::Plain),
                ("·", Role::Separator),
            ]
        );
    }

    #[test]
    fn notes_markers_and_vendors_carry_roles() {
        let vm = fixture("linux-arm64-apple-vm");
        let s = build(&vm, &UNICODE);
        assert_eq!(
            row(&s, "Vendor").lines[0].pieces(),
            [("Apple ", Role::Plain), ("†", Role::Marker)]
        );
        assert_eq!(
            row(&s, "Hypervisor").lines[0].pieces().last(),
            Some(&("topology as reported by guest", Role::Note))
        );
        assert_eq!(
            footnote(&vm, &UNICODE).unwrap().pieces(),
            [
                ("†", Role::Marker),
                (" from a built-in table: arm-midr@2026-09", Role::Plain)
            ]
        );
        let rosetta = build(&fixture("apple-m5-rosetta"), &UNICODE);
        assert_eq!(
            row(&rosetta, "Architecture").lines[0].pieces(),
            [
                ("arm64 ", Role::Plain),
                ("·", Role::Separator),
                (" ", Role::Plain),
                ("x86_64 binary running under Rosetta 2", Role::Note)
            ]
        );
        let amd = build(&fixture("linux-x86-gcp-epyc-7b12"), &UNICODE);
        assert_eq!(
            row(&amd, "Name").lines[0].pieces()[0].1,
            Role::Vendor(VendorTint::Amd)
        );
        let arm = build(&fixture("linux-arm64-oci-a1"), &UNICODE);
        assert_eq!(
            row(&arm, "Name").lines[0].pieces(),
            [
                ("Neoverse-N1", Role::Vendor(VendorTint::Other)),
                (" ", Role::Plain),
                ("†", Role::Marker)
            ]
        );
        let ascii = build(&fixture("apple-m5"), &ASCII);
        assert_eq!(
            row(&ascii, "Clusters").lines[0].pieces()[2],
            ("x", Role::Separator)
        );
    }

    #[test]
    fn wrapped_lines_keep_their_roles() {
        use crate::model::{Feature, FeatureGroup, FeatureGroupEntry};
        let mut cpu = Cpu::default();
        cpu.features.groups = vec![FeatureGroupEntry {
            group: FeatureGroup::Crypto,
            features: (0..20)
                .map(|i| Feature {
                    raw: format!("f{i}"),
                    name: format!("FEATURE{i:02}"),
                    family: None,
                })
                .collect(),
        }];
        let sections = build(&cpu, &UNICODE);
        let lines = &row(&sections, "Crypto").lines;
        assert!(lines.len() > 1, "{lines:?}");
        for line in lines {
            assert_eq!(
                line.pieces()[0].1,
                Role::Feature(FeatureGroup::Crypto),
                "{line}"
            );
        }
    }
}
