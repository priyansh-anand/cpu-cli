//! The default output: duf-style boxes, one per section, all the same width.
//!
//! `inner` is the number of columns between a box's two vertical borders.

use anstyle::Style;

use super::palette::Palette;
use super::view::{Body, Grid, Line, Pair, Section, grid_inner, grid_widths, span_width, width};

pub fn render(sections: &[Section], palette: Option<&Palette>) -> String {
    let inner = sections.iter().map(natural_inner).max().unwrap_or(0);
    let paint = Paint(palette);
    let mut out = String::new();
    for section in sections {
        match &section.body {
            Body::Pairs(pairs) => draw_pairs(&mut out, section.title, pairs, inner, paint),
            Body::Grid(grid) => draw_grid(&mut out, section.title, grid, inner, paint),
        }
    }
    out
}

/// `line` painted with `palette`, or its plain text when there is no palette.
pub fn paint_line(line: &Line, palette: Option<&Palette>) -> String {
    Paint(palette).line(line, 0, Style::new())
}

#[derive(Clone, Copy)]
struct Paint<'a>(Option<&'a Palette>);

impl Paint<'_> {
    fn styled(self, text: &str, style: Style) -> String {
        if self.0.is_some() && !text.is_empty() && style != Style::new() {
            format!("{}{text}{}", style.render(), style.render_reset())
        } else {
            text.to_string()
        }
    }

    fn chrome(self, text: &str) -> String {
        self.styled(text, self.0.map_or(Style::new(), |p| p.chrome))
    }

    fn title(self, text: &str) -> String {
        self.styled(text, self.0.map_or(Style::new(), |p| p.title))
    }

    fn label_style(self) -> Style {
        self.0.map_or(Style::new(), |p| p.label)
    }

    /// `line` painted piece by piece and padded to `w` columns; plain pieces get `base`. Padding
    /// goes outside the escape codes, so colour can never shift the layout.
    fn line(self, line: &Line, w: usize, base: Style) -> String {
        let mut out: String = line
            .pieces()
            .into_iter()
            .map(|(text, role)| {
                let style = self.0.and_then(|p| p.role(role)).unwrap_or(base);
                self.styled(text, style)
            })
            .collect();
        out.push_str(&" ".repeat(w.saturating_sub(width(line))));
        out
    }
}

fn label_width(pairs: &[Pair]) -> usize {
    pairs.iter().map(|p| width(&p.label)).max().unwrap_or(0)
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
    let bar = paint.chrome("│");
    out.push_str(&format!(
        "{} {} {}\n",
        paint.chrome("╭"),
        paint.title(title),
        paint.chrome(&format!("{dashes}╮"))
    ));
    let blank = Line::new();
    for pair in pairs {
        for (i, line) in pair.lines.iter().enumerate() {
            let label = if i == 0 { &pair.label } else { &blank };
            out.push_str(&format!(
                "{bar} {}  {} {bar}\n",
                paint.line(label, label_w, paint.label_style()),
                paint.line(line, value_w, Style::new())
            ));
        }
    }
    out.push_str(&format!(
        "{}\n",
        paint.chrome(&format!("╰{}╯", "─".repeat(inner)))
    ));
}

fn draw_grid(out: &mut String, title: &str, grid: &Grid, inner: usize, paint: Paint) {
    let mut w = grid_widths(title, grid);
    let last = w.len() - 1;
    w[last] += inner - grid_inner(&w);
    let seg = |i: usize| "─".repeat(w[i] + 2);
    let border = |join: &str| (0..w.len()).map(seg).collect::<Vec<_>>().join(join);
    let bar = paint.chrome("│");
    let label = paint.label_style();
    let row_line = |cells: &[&Line], header: bool| {
        cells
            .iter()
            .enumerate()
            .map(|(i, c)| {
                let base = if i == 0 || header {
                    label
                } else {
                    Style::new()
                };
                format!(" {} ", paint.line(c, w[i], base))
            })
            .collect::<Vec<_>>()
            .join(&bar)
    };
    let rest: String = (1..w.len()).map(|i| format!("┬{}", seg(i))).collect();
    let dashes = "─".repeat(w[0] - width(title));
    out.push_str(&format!(
        "{} {} {}\n",
        paint.chrome("╭"),
        paint.title(title),
        paint.chrome(&format!("{dashes}{rest}╮"))
    ));
    let corner = Line::plain(grid.corner);
    let header: Vec<&Line> = std::iter::once(&corner)
        .chain(grid.columns.iter())
        .collect();
    out.push_str(&format!("{bar}{}{bar}\n", row_line(&header, true)));
    out.push_str(&format!(
        "{}\n",
        paint.chrome(&format!("├{}┤", border("┼")))
    ));
    let blank = Line::new();
    for row in &grid.rows {
        if row.span {
            let text = row.cells.first().unwrap_or(&blank);
            out.push_str(&format!(
                "{bar} {} {bar} {} {bar}\n",
                paint.line(&row.label, w[0], label),
                paint.line(text, span_width(&w), Style::new())
            ));
        } else {
            let cells: Vec<&Line> = std::iter::once(&row.label)
                .chain(row.cells.iter())
                .collect();
            out.push_str(&format!("{bar}{}{bar}\n", row_line(&cells, false)));
        }
    }
    out.push_str(&format!(
        "{}\n",
        paint.chrome(&format!("╰{}╯", border("┴")))
    ));
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::render::palette::DARK_256;
    use crate::render::view::{UNICODE, build};
    use crate::test_support::fixture;
    use anstyle::Style;

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
            render(&build(&fixture("apple-m5"), &UNICODE), None),
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
            let text = render(&build(&fixture(name), &UNICODE), None);
            let widths: std::collections::BTreeSet<usize> = text
                .lines()
                .filter(|l| l.starts_with(['╭', '│', '├', '╰']))
                .map(width)
                .collect();
            assert_eq!(widths.len(), 1, "{name}:\n{text}");
        }
    }

    #[test]
    fn colour_never_changes_the_layout() {
        use crate::render::palette::{ANSI_16, LIGHT_256};
        let sections = build(&fixture("apple-m5"), &UNICODE);
        for palette in [&DARK_256, &LIGHT_256, &ANSI_16] {
            let coloured = render(&sections, Some(palette));
            assert!(coloured.contains("\x1b[1m"), "titles are bold");
            assert_eq!(strip_ansi(&coloured), render(&sections, None));
        }
    }

    #[test]
    fn roles_are_painted_with_the_palette() {
        let text = render(
            &build(&fixture("linux-x86-intel-hybrid"), &UNICODE),
            Some(&DARK_256),
        );
        let painted = |style: Style, piece: &str| {
            format!("{}{piece}{}", style.render(), style.render_reset())
        };
        for (style, piece) in [
            (DARK_256.performance, "Performance"),
            (DARK_256.efficiency, "Efficiency"),
            (DARK_256.levels[2], "L3"),
            (DARK_256.simd, "AVX2"),
            (DARK_256.unit, "GHz"),
            (DARK_256.intel, "12th Gen Intel(R) Core(TM) i7-12700K"),
            (DARK_256.title, "Cache"),
            (DARK_256.chrome, "│"),
        ] {
            assert!(
                text.contains(&painted(style, piece)),
                "{piece} not painted:\n{text}"
            );
        }
    }
}
