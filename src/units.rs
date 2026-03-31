//! Units with human-friendly display. Values are stored raw (bytes, hertz) so JSON stays exact.

use std::fmt;

use serde::Serialize;

/// A size in bytes. Displays an exact whole unit when one exists (`192 KiB`, `1280 KiB`), else one decimal (`1.5 KiB`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize)]
#[serde(transparent)]
pub struct Bytes(pub u64);

/// A frequency in hertz. Displays as `3.50 GHz`, `800 MHz`, `12 Hz`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize)]
#[serde(transparent)]
pub struct Hertz(pub u64);

impl fmt::Display for Bytes {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        const UNITS: [(u64, &str); 3] = [(1 << 30, "GiB"), (1 << 20, "MiB"), (1 << 10, "KiB")];
        // An exact whole number in the largest unit that divides evenly reads best: 1280 KiB,
        // not a rounded 1.2 MiB.
        if let Some((size, unit)) = UNITS
            .iter()
            .find(|(size, _)| self.0 >= *size && self.0 % size == 0)
        {
            return write!(f, "{} {unit}", self.0 / size);
        }
        match UNITS.iter().find(|(size, _)| self.0 >= *size) {
            Some((size, unit)) => write!(f, "{:.1} {unit}", self.0 as f64 / *size as f64),
            None => write!(f, "{} B", self.0),
        }
    }
}

impl fmt::Display for Hertz {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self.0 {
            n if n >= 1_000_000_000 => write!(f, "{:.2} GHz", n as f64 / 1e9),
            n if n >= 1_000_000 => write!(f, "{} MHz", (n + 500_000) / 1_000_000),
            n => write!(f, "{n} Hz"),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bytes_use_binary_units() {
        assert_eq!(Bytes(196_608).to_string(), "192 KiB");
        assert_eq!(Bytes(16_777_216).to_string(), "16 MiB");
        assert_eq!(Bytes(6_291_456).to_string(), "6 MiB");
        assert_eq!(Bytes(3 << 30).to_string(), "3 GiB");
        assert_eq!(Bytes(1536).to_string(), "1.5 KiB");
        assert_eq!(Bytes(512).to_string(), "512 B");
    }

    #[test]
    fn bytes_prefer_an_exact_whole_unit() {
        assert_eq!(Bytes(1_310_720).to_string(), "1280 KiB");
        assert_eq!(Bytes(25 << 20).to_string(), "25 MiB");
        assert_eq!(Bytes(2 << 20).to_string(), "2 MiB");
    }

    #[test]
    fn hertz_use_decimal_units() {
        assert_eq!(Hertz(3_504_000_000).to_string(), "3.50 GHz");
        assert_eq!(Hertz(800_000_000).to_string(), "800 MHz");
        assert_eq!(Hertz(12).to_string(), "12 Hz");
    }

    #[test]
    fn units_serialize_as_plain_numbers() {
        assert_eq!(serde_json::to_string(&Bytes(1024)).unwrap(), "1024");
        assert_eq!(serde_json::to_string(&Hertz(5)).unwrap(), "5");
    }
}
