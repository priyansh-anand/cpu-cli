//! `--plain`: aligned text with no box-drawing and no colour. ASCII-only when built with
//! [`ASCII`](super::view::ASCII) glyphs, so it is safe for pipes, logs and any locale.

use super::view::{Body, Line, Section, column_widths, pad, width};

/// `s` with every non-ASCII character replaced by `?`, so plain output is ASCII whatever the OS reported.
fn ascii(s: &str) -> String {
    s.chars()
        .map(|c| if c.is_ascii() { c } else { '?' })
        .collect()
}

pub fn render(sections: &[Section]) -> String {
    let mut out: Vec<String> = Vec::new();
    for (i, section) in sections.iter().enumerate() {
        if i > 0 {
            out.push(String::new());
        }
        out.push(section.title.to_string());
        match &section.body {
            Body::Pairs(pairs) => {
                let label_width = pairs.iter().map(|p| width(&p.label)).max().unwrap_or(0);
                for pair in pairs {
                    for (j, line) in pair.lines.iter().enumerate() {
                        let label = if j == 0 { pair.label.as_str() } else { "" };
                        let line = ascii(line);
                        out.push(
                            format!("  {}  {line}", pad(label, label_width))
                                .trim_end()
                                .to_string(),
                        );
                    }
                }
            }
            Body::Grid(grid) => {
                let cells: Vec<Vec<String>> = grid
                    .table()
                    .iter()
                    .map(|row| row.iter().map(|c| ascii(c)).collect())
                    .collect();
                let table: Vec<Vec<&str>> = cells
                    .iter()
                    .map(|row| row.iter().map(String::as_str).collect())
                    .collect();
                let mut widths = column_widths(&table);
                for row in grid.rows.iter().filter(|r| r.span) {
                    widths[0] = widths[0].max(width(&row.label));
                }
                let aligned = |cells: &[&str]| {
                    let padded: Vec<String> =
                        cells.iter().zip(&widths).map(|(c, w)| pad(c, *w)).collect();
                    format!("  {}", padded.join("  ")).trim_end().to_string()
                };
                out.push(aligned(&table[0]));
                let mut data = table[1..].iter();
                for row in &grid.rows {
                    if row.span {
                        let text = ascii(row.cells.first().map_or("", Line::as_str));
                        out.push(
                            format!("  {}  {text}", pad(&ascii(&row.label), widths[0]))
                                .trim_end()
                                .to_string(),
                        );
                    } else if let Some(cells) = data.next() {
                        out.push(aligned(cells));
                    }
                }
            }
        }
    }
    let mut text = out.join("\n");
    text.push('\n');
    text
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::render::view::{ASCII, build};
    use crate::test_support::fixture;

    const APPLE_M5: &str = "\
Identity
  Name          Apple M5
  Vendor        Apple
  Architecture  arm64

Topology
  Cores     10 physical, 10 logical, no SMT
  Clusters  4 x Super, 6 x Efficiency

Clocks
       Super     Efficiency
  Max  4.46 GHz  3.05 GHz

Cache
  Level  Super             Efficiency
  L1i    192 KiB / core    128 KiB / core
  L1d    128 KiB / core    64 KiB / core
  L2     16 MiB / 4 cores  6 MiB / 6 cores

Features
  SIMD      NEON, FP16, BF16, I8MM, DotProd, FHM, SME, SME2, SME2.1
  Crypto    AES, PMULL, SHA1, SHA256, SHA512, SHA3, CRC32
  Security  PAC, BTI, MTE, DIT, SB, CSV2, CSV3
  Other     LSE, LSE2
";

    const SPARSE: &str = "\
Identity
  Name          Apple M1
  Vendor        Apple
  Architecture  arm64

Topology
  Cores  8 physical, 8 logical, no SMT
";

    #[test]
    fn non_ascii_values_are_replaced_so_plain_stays_ascii() {
        let mut cpu = crate::model::Cpu::default();
        cpu.identity.name = Some(crate::model::Fact::derived("Apple M5 ™".to_string()));
        assert_eq!(
            render(&build(&cpu, &ASCII)),
            "Identity\n  Name  Apple M5 ?\n"
        );
    }

    #[test]
    fn apple_m5_plain_output() {
        assert_eq!(render(&build(&fixture("apple-m5"), &ASCII)), APPLE_M5);
    }

    #[test]
    fn sparse_plain_output() {
        assert_eq!(render(&build(&fixture("sparse-mac"), &ASCII)), SPARSE);
    }

    #[test]
    fn spanning_rows_follow_the_label_column() {
        let text = render(&build(&fixture("linux-x86-intel-hybrid"), &ASCII));
        assert!(text.contains("\n  L2     1280 KiB / core  2 MiB / 4 cores\n  L3     25 MiB shared by all 12 cores\n"), "{text}");
    }
}
