//! The default output: duf-style boxes, one per section, all the same width.
//!
//! `inner` is the number of columns between a box's two vertical borders.

use anstyle::{AnsiColor, Color, Style};

use super::view::{Body, Grid, Pair, Section, column_widths, pad, width};

const TITLE: Style = Style::new().bold();
const LABEL: Style = Style::new().fg_color(Some(Color::Ansi(AnsiColor::Cyan)));

pub fn render(sections: &[Section], color: bool) -> String {
    let inner = sections.iter().map(natural_inner).max().unwrap_or(0);
    let paint = Paint(color);
    let mut out = String::new();
    for section in sections {
        match &section.body {
            Body::Pairs(pairs) => draw_pairs(&mut out, section.title, pairs, inner, paint),
            Body::Grid(grid) => draw_grid(&mut out, section.title, grid, inner, paint),
        }
    }
    out
}

#[derive(Clone, Copy)]
struct Paint(bool);

impl Paint {
    /// `text` styled and padded to `w` columns. Padding goes outside the escape codes, so colour
    /// can never shift the layout.
    fn cell(self, text: &str, w: usize, style: Style) -> String {
        let fill = " ".repeat(w.saturating_sub(width(text)));
        if self.0 && !text.is_empty() && style != Style::new() {
            format!("{}{text}{}{fill}", style.render(), style.render_reset())
        } else {
            format!("{text}{fill}")
        }
    }
}

fn label_width(pairs: &[Pair]) -> usize {
    pairs.iter().map(|p| width(p.label)).max().unwrap_or(0)
}

/// Column widths for a grid; the first column is widened so the title fits in its top border.
/// Column widths for a grid: the first column fits the title in its top border, and the last
/// column grows until every spanning row's text fits.
fn grid_widths(title: &str, grid: &Grid) -> Vec<usize> {
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
fn span_width(widths: &[usize]) -> usize {
    widths[1..].iter().sum::<usize>() + 3 * (widths.len() - 2)
}

fn grid_inner(widths: &[usize]) -> usize {
    widths.iter().map(|w| w + 2).sum::<usize>() + widths.len() - 1
}

fn natural_inner(section: &Section) -> usize {
    match &section.body {
        Body::Pairs(pairs) => {
            let value_width = pairs
                .iter()
                .flat_map(|p| &p.lines)
                .map(|l| width(l))
                .max()
                .unwrap_or(0);
            (label_width(pairs) + value_width + 4).max(width(section.title) + 3)
        }
        Body::Grid(grid) => grid_inner(&grid_widths(section.title, grid)),
    }
}

fn draw_pairs(out: &mut String, title: &str, pairs: &[Pair], inner: usize, paint: Paint) {
    let label_w = label_width(pairs);
    let value_w = inner - label_w - 4;
    let dashes = "─".repeat(inner - width(title) - 2);
    out.push_str(&format!("╭ {} {dashes}╮\n", paint.cell(title, 0, TITLE)));
    for pair in pairs {
        for (i, line) in pair.lines.iter().enumerate() {
            let label = if i == 0 { pair.label } else { "" };
            out.push_str(&format!(
                "│ {}  {} │\n",
                paint.cell(label, label_w, LABEL),
                pad(line, value_w)
            ));
        }
    }
    out.push_str(&format!("╰{}╯\n", "─".repeat(inner)));
}

fn draw_grid(out: &mut String, title: &str, grid: &Grid, inner: usize, paint: Paint) {
    let mut w = grid_widths(title, grid);
    let last = w.len() - 1;
    w[last] += inner - grid_inner(&w);
    let seg = |i: usize| "─".repeat(w[i] + 2);
    let border = |join: &str| (0..w.len()).map(seg).collect::<Vec<_>>().join(join);
    let line = |cells: &[&str], header: bool| {
        cells
            .iter()
            .enumerate()
            .map(|(i, c)| {
                let style = if i == 0 || header {
                    LABEL
                } else {
                    Style::new()
                };
                format!(" {} ", paint.cell(c, w[i], style))
            })
            .collect::<Vec<_>>()
            .join("│")
    };
    let rest: String = (1..w.len()).map(|i| format!("┬{}", seg(i))).collect();
    let dashes = "─".repeat(w[0] - width(title));
    out.push_str(&format!(
        "╭ {} {dashes}{rest}╮\n",
        paint.cell(title, 0, TITLE)
    ));
    out.push_str(&format!("│{}│\n", line(&grid.header(), true)));
    out.push_str(&format!("├{}┤\n", border("┼")));
    for row in &grid.rows {
        if row.span {
            let text = row.cells.first().map_or("", String::as_str);
            out.push_str(&format!(
                "│ {} │ {} │\n",
                paint.cell(&row.label, w[0], LABEL),
                pad(text, span_width(&w))
            ));
        } else {
            let cells: Vec<&str> = std::iter::once(row.label.as_str())
                .chain(row.cells.iter().map(String::as_str))
                .collect();
            out.push_str(&format!("│{}│\n", line(&cells, false)));
        }
    }
    out.push_str(&format!("╰{}╯\n", border("┴")));
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::render::view::{UNICODE, build};
    use crate::test_support::fixture;

    const APPLE_M5: &str = "\
╭ Identity ────────────────────────────────────────────────────────╮
│ Name          Apple M5                                           │
│ Vendor        Apple                                              │
│ Architecture  arm64                                              │
╰──────────────────────────────────────────────────────────────────╯
╭ Topology ────────────────────────────────────────────────────────╮
│ Cores     10 physical · 10 logical · no SMT                      │
│ Clusters  4 × Super · 6 × Efficiency                             │
╰──────────────────────────────────────────────────────────────────╯
╭ Clocks ─┬──────────┬─────────────────────────────────────────────╮
│         │ Super    │ Efficiency                                  │
├─────────┼──────────┼─────────────────────────────────────────────┤
│ Max     │ 4.46 GHz │ 3.05 GHz                                    │
╰─────────┴──────────┴─────────────────────────────────────────────╯
╭ Cache ─┬──────────────────┬──────────────────────────────────────╮
│ Level  │ Super            │ Efficiency                           │
├────────┼──────────────────┼──────────────────────────────────────┤
│ L1i    │ 192 KiB / core   │ 128 KiB / core                       │
│ L1d    │ 128 KiB / core   │ 64 KiB / core                        │
│ L2     │ 16 MiB / 4 cores │ 6 MiB / 6 cores                      │
╰────────┴──────────────────┴──────────────────────────────────────╯
╭ Features ────────────────────────────────────────────────────────╮
│ SIMD      NEON · FP16 · BF16 · I8MM · DotProd · FHM · SME · SME2 │
│           SME2.1                                                 │
│ Crypto    AES · PMULL · SHA1 · SHA256 · SHA512 · SHA3 · CRC32    │
│ Security  PAC · BTI · MTE · DIT · SB · CSV2 · CSV3               │
│ Other     LSE · LSE2                                             │
╰──────────────────────────────────────────────────────────────────╯
";

    fn strip_ansi(s: &str) -> String {
        let mut out = String::new();
        let mut chars = s.chars();
        while let Some(c) = chars.next() {
            if c == '\x1b' {
                for c in chars.by_ref() {
                    if c == 'm' {
                        break;
                    }
                }
            } else {
                out.push(c);
            }
        }
        out
    }

    #[test]
    fn apple_m5_boxed_output() {
        assert_eq!(
            render(&build(&fixture("apple-m5"), &UNICODE), false),
            APPLE_M5
        );
    }

    #[test]
    fn every_line_has_the_same_width() {
        for name in [
            "apple-m5",
            "apple-m2-pro",
            "sparse-mac",
            "linux-x86-intel-hybrid",
            "linux-x86-amd-gce",
            "linux-arm64-apple-vm",
        ] {
            let text = render(&build(&fixture(name), &UNICODE), false);
            let widths: std::collections::BTreeSet<usize> = text.lines().map(width).collect();
            assert_eq!(widths.len(), 1, "{name}:\n{text}");
        }
    }

    #[test]
    fn colour_never_changes_the_layout() {
        let sections = build(&fixture("apple-m5"), &UNICODE);
        let coloured = render(&sections, true);
        assert!(coloured.contains("\x1b[1m"), "titles are bold");
        assert_eq!(strip_ansi(&coloured), render(&sections, false));
    }
}
