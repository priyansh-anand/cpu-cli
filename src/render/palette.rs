//! Colour palettes: the style each [`Role`] gets in boxed output.
//!
//! Numbers are xterm-256 colour indices. Dark and light 256-colour palettes are tuned for their
//! backgrounds; the 16-colour palette uses the terminal's own colours, which follow its theme.

use anstyle::{Ansi256Color, AnsiColor, Color, Style};

use super::view::{Role, VendorTint};
use crate::model::{CoreKind, FeatureGroup};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Theme {
    Dark256,
    Light256,
    Ansi16,
}

impl Theme {
    pub fn palette(self) -> &'static Palette {
        match self {
            Theme::Dark256 => &DARK_256,
            Theme::Light256 => &LIGHT_256,
            Theme::Ansi16 => &ANSI_16,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Palette {
    pub title: Style,
    pub label: Style,
    /// Box drawing: borders and column separators.
    pub chrome: Style,
    pub number: Style,
    pub unit: Style,
    pub separator: Style,
    pub performance: Style,
    pub efficiency: Style,
    pub uniform: Style,
    pub simd: Style,
    pub crypto: Style,
    pub virtualization: Style,
    pub security: Style,
    pub other_feature: Style,
    /// Cache levels 1, 2 and 3.
    pub levels: [Style; 3],
    pub intel: Style,
    pub amd: Style,
    pub apple: Style,
    pub other_vendor: Style,
    pub note: Style,
    pub marker: Style,
}

impl Palette {
    /// The style for `role`; `None` for plain text, which takes the style of its surroundings.
    pub fn role(&self, role: Role) -> Option<Style> {
        Some(match role {
            Role::Plain => return None,
            Role::Number => self.number,
            Role::Unit => self.unit,
            Role::Separator => self.separator,
            Role::Kind(CoreKind::Performance) => self.performance,
            Role::Kind(CoreKind::Efficiency) => self.efficiency,
            Role::Kind(CoreKind::Uniform) => self.uniform,
            Role::Feature(FeatureGroup::Simd) => self.simd,
            Role::Feature(FeatureGroup::Crypto) => self.crypto,
            Role::Feature(FeatureGroup::Virtualization) => self.virtualization,
            Role::Feature(FeatureGroup::Security) => self.security,
            Role::Feature(FeatureGroup::Other) => self.other_feature,
            Role::Level(n) => self.levels[usize::from(n.clamp(1, 3)) - 1],
            Role::Vendor(VendorTint::Intel) => self.intel,
            Role::Vendor(VendorTint::Amd) => self.amd,
            Role::Vendor(VendorTint::Apple) => self.apple,
            Role::Vendor(VendorTint::Other) => self.other_vendor,
            Role::Note => self.note,
            Role::Marker => self.marker,
        })
    }
}

const BOLD: Style = Style::new().bold();

const fn c256(n: u8) -> Style {
    Style::new().fg_color(Some(Color::Ansi256(Ansi256Color(n))))
}

const fn c16(c: AnsiColor) -> Style {
    Style::new().fg_color(Some(Color::Ansi(c)))
}

/// Scheme B "Vivid" for dark backgrounds.
pub const DARK_256: Palette = Palette {
    title: BOLD,
    label: c256(37),
    chrome: c256(244),
    number: BOLD,
    unit: c256(244),
    separator: c256(244),
    performance: c256(209).bold(),
    efficiency: c256(71).bold(),
    uniform: BOLD,
    simd: c256(75),
    crypto: c256(179),
    virtualization: c256(141),
    security: c256(168),
    other_feature: c256(246),
    levels: [c256(116).bold(), c256(74).bold(), c256(68).bold()],
    intel: c256(33).bold(),
    amd: c256(160).bold(),
    apple: c256(252).bold(),
    other_vendor: BOLD,
    note: c256(214),
    marker: c256(214),
};

/// The same roles in darker, saturated shades for light backgrounds.
pub const LIGHT_256: Palette = Palette {
    title: BOLD,
    label: c256(30),
    chrome: c256(244),
    number: BOLD,
    unit: c256(243),
    separator: c256(245),
    performance: c256(166).bold(),
    efficiency: c256(28).bold(),
    uniform: BOLD,
    simd: c256(25),
    crypto: c256(136),
    virtualization: c256(91),
    security: c256(161),
    other_feature: c256(242),
    levels: [c256(30).bold(), c256(25).bold(), c256(18).bold()],
    intel: c256(25).bold(),
    amd: c256(160).bold(),
    apple: c256(238).bold(),
    other_vendor: BOLD,
    note: c256(130),
    marker: c256(130),
};

/// Terminals without 256 colours: the 16 ANSI colours, remapped by the terminal's theme.
pub const ANSI_16: Palette = Palette {
    title: BOLD,
    label: c16(AnsiColor::Cyan),
    chrome: c16(AnsiColor::BrightBlack),
    number: BOLD,
    unit: c16(AnsiColor::BrightBlack),
    separator: c16(AnsiColor::BrightBlack),
    performance: c16(AnsiColor::Yellow).bold(),
    efficiency: c16(AnsiColor::Green).bold(),
    uniform: BOLD,
    simd: c16(AnsiColor::Blue),
    crypto: c16(AnsiColor::Yellow),
    virtualization: c16(AnsiColor::Magenta),
    security: c16(AnsiColor::Red),
    other_feature: Style::new(),
    levels: [
        c16(AnsiColor::Cyan).bold(),
        c16(AnsiColor::Blue).bold(),
        c16(AnsiColor::Magenta).bold(),
    ],
    intel: c16(AnsiColor::Blue).bold(),
    amd: c16(AnsiColor::Red).bold(),
    apple: BOLD,
    other_vendor: BOLD,
    note: c16(AnsiColor::Yellow),
    marker: c16(AnsiColor::Yellow),
};

#[cfg(test)]
mod tests {
    use super::*;
    use anstyle::{Ansi256Color, Color};

    /// xterm-256 RGB for the 6x6x6 cube and the grey ramp. The 16 system colours depend on the
    /// terminal theme, so a 256 palette must never use them.
    fn rgb(n: u8) -> [f64; 3] {
        let v = |x: u8| f64::from(x) / 255.0;
        match n {
            16..=231 => {
                let i = n - 16;
                let step = |c: u8| if c == 0 { 0 } else { 55 + 40 * c };
                [v(step(i / 36)), v(step(i / 6 % 6)), v(step(i % 6))]
            }
            232..=255 => {
                let g = 8 + 10 * (n - 232);
                [v(g), v(g), v(g)]
            }
            _ => panic!("system colour {n} in a 256-colour palette"),
        }
    }

    /// WCAG relative luminance.
    fn luminance([r, g, b]: [f64; 3]) -> f64 {
        let lin = |c: f64| {
            if c <= 0.03928 {
                c / 12.92
            } else {
                ((c + 0.055) / 1.055).powf(2.4)
            }
        };
        0.2126 * lin(r) + 0.7152 * lin(g) + 0.0722 * lin(b)
    }

    fn contrast(a: f64, b: f64) -> f64 {
        let (hi, lo) = if a > b { (a, b) } else { (b, a) };
        (hi + 0.05) / (lo + 0.05)
    }

    /// Every named style; destructuring means a new field cannot be left out of these tests.
    fn styles(p: &Palette) -> Vec<(&'static str, Style)> {
        let Palette {
            title,
            label,
            chrome,
            number,
            unit,
            separator,
            performance,
            efficiency,
            uniform,
            simd,
            crypto,
            virtualization,
            security,
            other_feature,
            levels,
            intel,
            amd,
            apple,
            other_vendor,
            note,
            marker,
        } = *p;
        vec![
            ("title", title),
            ("label", label),
            ("chrome", chrome),
            ("number", number),
            ("unit", unit),
            ("separator", separator),
            ("performance", performance),
            ("efficiency", efficiency),
            ("uniform", uniform),
            ("simd", simd),
            ("crypto", crypto),
            ("virtualization", virtualization),
            ("security", security),
            ("other_feature", other_feature),
            ("level1", levels[0]),
            ("level2", levels[1]),
            ("level3", levels[2]),
            ("intel", intel),
            ("amd", amd),
            ("apple", apple),
            ("other_vendor", other_vendor),
            ("note", note),
            ("marker", marker),
        ]
    }

    fn check_contrast(p: &Palette, background: f64) {
        for (name, style) in styles(p) {
            match style.get_fg_color() {
                None => {}
                Some(Color::Ansi256(Ansi256Color(n))) => {
                    let ratio = contrast(luminance(rgb(n)), background);
                    assert!(ratio >= 3.0, "{name} (colour {n}) is only {ratio:.2}:1");
                }
                Some(other) => panic!("{name} uses {other:?}, not a 256-colour index"),
            }
        }
    }

    #[test]
    fn xterm_values() {
        assert_eq!(rgb(209), [1.0, 135.0 / 255.0, 95.0 / 255.0]);
        assert_eq!(rgb(244), [128.0 / 255.0; 3]);
    }

    #[test]
    fn dark_palette_reads_on_black() {
        check_contrast(&DARK_256, 0.0);
    }

    #[test]
    fn light_palette_reads_on_white() {
        check_contrast(&LIGHT_256, 1.0);
    }

    #[test]
    fn sixteen_colour_palette_follows_the_terminal_theme() {
        for (name, style) in styles(&ANSI_16) {
            assert!(
                !matches!(
                    style.get_fg_color(),
                    Some(Color::Ansi256(_) | Color::Rgb(_))
                ),
                "{name} must be one of the 16 ANSI colours"
            );
        }
    }

    #[test]
    fn roles_map_to_their_styles() {
        use crate::model::{CoreKind, FeatureGroup};
        assert_eq!(DARK_256.role(Role::Plain), None);
        assert_eq!(
            DARK_256.role(Role::Kind(CoreKind::Efficiency)),
            Some(DARK_256.efficiency)
        );
        assert_eq!(DARK_256.role(Role::Level(3)), Some(DARK_256.levels[2]));
        assert_eq!(
            LIGHT_256.role(Role::Feature(FeatureGroup::Security)),
            Some(LIGHT_256.security)
        );
        assert_eq!(
            ANSI_16.role(Role::Vendor(VendorTint::Amd)),
            Some(ANSI_16.amd)
        );
        assert_eq!(Theme::Light256.palette(), &LIGHT_256);
    }
}
