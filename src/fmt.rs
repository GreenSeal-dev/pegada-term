//! Units and colours for the CLI. The hooks implement the same unit rules in shell.

use std::io::IsTerminal;

use crate::paths;

/// mJ / J / kJ / Wh / kWh.
pub fn energy(mj: u64) -> String {
    match mj {
        0..=999 => format!("{mj} mJ"),
        1_000..=9_999 => format!("{}.{} J", mj / 1000, mj % 1000 / 100),
        10_000..=999_999 => format!("{} J", mj / 1000),
        1_000_000..=9_999_999 => format!("{}.{} kJ", mj / 1_000_000, mj % 1_000_000 / 100_000),
        10_000_000..=99_999_999 => format!("{} kJ", mj / 1_000_000),
        _ => watt_hours(mj),
    }
}

/// Always Wh or kWh, for totals.
pub fn watt_hours(mj: u64) -> String {
    let wh = mj as f64 / 3_600_000.0;
    if wh < 10.0 {
        format!("{wh:.2} Wh")
    } else if wh < 1000.0 {
        format!("{wh:.1} Wh")
    } else {
        format!("{:.2} kWh", wh / 1000.0)
    }
}

pub fn power(mw: u64) -> String {
    if mw < 1000 {
        format!("{mw} mW")
    } else {
        format!("{}.{} W", mw / 1000, mw % 1000 / 100)
    }
}

/// ms / s / m s / h m.
pub fn duration(ms: u64) -> String {
    let s = ms / 1000;
    match ms {
        0..=999 => format!("{ms} ms"),
        1_000..=59_999 => format!("{}.{} s", s, ms % 1000 / 100),
        60_000..=3_599_999 => format!("{}m {:02}s", s / 60, s % 60),
        _ => format!("{}h {:02}m", s / 3600, s % 3600 / 60),
    }
}

pub fn locale_is_utf8() -> bool {
    let locale = ["LC_ALL", "LC_CTYPE", "LANG"]
        .iter()
        .find_map(|v| paths::var(v))
        .unwrap_or_default()
        .to_ascii_lowercase();
    locale.contains("utf-8") || locale.contains("utf8")
}

#[derive(Clone, Copy)]
pub struct Style {
    pub color: bool,
    pub utf8: bool,
}

pub const GREEN: u8 = 71;
pub const AMBER: u8 = 214;
pub const RED: u8 = 203;

impl Style {
    pub fn detect() -> Self {
        Self {
            color: paths::var("NO_COLOR").is_none() && std::io::stdout().is_terminal(),
            utf8: locale_is_utf8() && paths::var("PEGADA_TERM_ASCII").is_none_or(|v| v == "0"),
        }
    }

    fn wrap(&self, code: &str, text: &str) -> String {
        if self.color {
            format!("\x1b[{code}m{text}\x1b[0m")
        } else {
            text.to_string()
        }
    }
    pub fn dim(&self, text: &str) -> String {
        self.wrap("2", text)
    }
    pub fn bold(&self, text: &str) -> String {
        self.wrap("1", text)
    }
    pub fn fg(&self, color: u8, text: &str) -> String {
        self.wrap(&format!("38;5;{color}"), text)
    }
    pub fn bolt(&self) -> &'static str {
        if self.utf8 {
            "⚡"
        } else {
            "*"
        }
    }
    pub fn ok(&self) -> String {
        self.fg(GREEN, if self.utf8 { "✓" } else { "ok" })
    }
    pub fn warn(&self) -> String {
        self.fg(AMBER, "!")
    }
    pub fn bad(&self) -> String {
        self.fg(RED, if self.utf8 { "✗" } else { "x" })
    }
    /// A bar of `width` cells with the first `lit` cells filled.
    pub fn bar(&self, lit: usize, width: usize) -> (String, String) {
        let (full, empty) = if self.utf8 {
            ("█", "░")
        } else {
            ("#", ".")
        };
        let lit = lit.min(width);
        (full.repeat(lit), empty.repeat(width - lit))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn energy_units() {
        assert_eq!(energy(0), "0 mJ");
        assert_eq!(energy(999), "999 mJ");
        assert_eq!(energy(1_250), "1.2 J");
        assert_eq!(energy(38_400), "38 J");
        assert_eq!(energy(142_000), "142 J");
        assert_eq!(energy(3_200_000), "3.2 kJ");
        assert_eq!(energy(42_000_000), "42 kJ");
        assert_eq!(energy(360_000_000), "100.0 Wh");
        assert_eq!(energy(7_200_000_000), "2.00 kWh");
    }

    #[test]
    fn power_and_duration_units() {
        assert_eq!(power(850), "850 mW");
        assert_eq!(power(11_540), "11.5 W");
        assert_eq!(duration(42), "42 ms");
        assert_eq!(duration(12_400), "12.4 s");
        assert_eq!(duration(192_000), "3m 12s");
        assert_eq!(duration(3_900_000), "1h 05m");
    }
}
