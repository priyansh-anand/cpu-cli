//! `--plain`: aligned text with no box-drawing and no colour. ASCII-only when built with
//! [`ASCII`](super::view::ASCII) glyphs, so it is safe for pipes, logs and any locale.

use super::view::{Body, Section, column_widths, pad, width};

pub fn render(sections: &[Section]) -> String {
    let mut out: Vec<String> = Vec::new();
    for (i, section) in sections.iter().enumerate() {
        if i > 0 {
            out.push(String::new());
        }
        out.push(section.title.to_string());
        match &section.body {
            Body::Pairs(pairs) => {
                let label_width = pairs.iter().map(|p| width(p.label)).max().unwrap_or(0);
                for pair in pairs {
                    for (j, line) in pair.lines.iter().enumerate() {
                        let label = if j == 0 { pair.label } else { "" };
                        out.push(
                            format!("  {}  {line}", pad(label, label_width))
                                .trim_end()
                                .to_string(),
                        );
                    }
                }
            }
            Body::Grid(grid) => {
                let table = grid.table();
                let widths = column_widths(&table);
                for cells in &table {
                    let line: Vec<String> =
                        cells.iter().zip(&widths).map(|(c, w)| pad(c, *w)).collect();
                    out.push(format!("  {}", line.join("  ")).trim_end().to_string());
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
    fn apple_m5_plain_output() {
        assert_eq!(render(&build(&fixture("apple-m5"), &ASCII)), APPLE_M5);
    }

    #[test]
    fn sparse_plain_output() {
        assert_eq!(render(&build(&fixture("sparse-mac"), &ASCII)), SPARSE);
    }
}
