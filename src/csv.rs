//! EnergiBridge CSV: `Delta,Time,<sorted keys>`.
//!
//! A row is printed `Delta` ms after the epoch-ms timestamp in `Time`, so the
//! values in it belong to `Time + Delta`.

use crate::integrate::{Reading, Sample};

enum Kind {
    Watts(usize),
    Joules(Vec<usize>),
}

pub struct Columns {
    delta: usize,
    time: usize,
    kind: Kind,
    pub source: &'static str,
}

impl Columns {
    /// Picks the energy columns for this platform; `None` if the header has none.
    pub fn from_header(header: &str) -> Option<Self> {
        let names: Vec<&str> = header.trim_end().split(',').map(str::trim).collect();
        let idx = |name: &str| names.iter().position(|c| *c == name);
        let delta = idx("Delta")?;
        let time = idx("Time")?;
        let (kind, source) = if let Some(i) = idx("SYSTEM_POWER (Watts)") {
            (Kind::Watts(i), "smc-system")
        } else if let Some(i) = idx("CPU_POWER (Watts)") {
            (Kind::Watts(i), "smc-cpu")
        } else if let Some(pkg) = idx("PACKAGE_ENERGY (J)") {
            match idx("DRAM_ENERGY (J)") {
                Some(dram) => (Kind::Joules(vec![pkg, dram]), "rapl-package+dram"),
                None => (Kind::Joules(vec![pkg]), "rapl-package"),
            }
        } else {
            (
                Kind::Joules(vec![idx("CPU_ENERGY (J)")?]),
                "rapl-amd-package",
            )
        };
        Some(Self {
            delta,
            time,
            kind,
            source,
        })
    }

    pub fn parse_row(&self, row: &str) -> Option<Sample> {
        let fields: Vec<&str> = row.trim_end().split(',').collect();
        let num = |i: usize| fields.get(i)?.trim().parse::<f64>().ok();
        let t_ms = (num(self.time)? + num(self.delta)?) as u64;
        let reading = match &self.kind {
            Kind::Watts(i) => Reading::Watts(num(*i)?),
            Kind::Joules(cols) => {
                Reading::Joules(cols.iter().map(|i| num(*i)).collect::<Option<_>>()?)
            }
        };
        Some(Sample { t_ms, reading })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn apple_silicon_header() {
        let c = Columns::from_header(
            "Delta,Time,CPU_FREQUENCY_0,CPU_USAGE_0,SYSTEM_POWER (Watts),TOTAL_MEMORY\n",
        )
        .unwrap();
        assert_eq!(c.source, "smc-system");
        let s = c
            .parse_row("505,1790762787650,3228,100,10.978314399719238,34359738368\n")
            .unwrap();
        assert_eq!(s.t_ms, 1_790_762_788_155);
        assert_eq!(s.reading, Reading::Watts(10.978314399719238));
    }

    #[test]
    fn system_power_wins_over_cpu_power() {
        let c = Columns::from_header("Delta,Time,CPU_POWER (Watts),SYSTEM_POWER (Watts)").unwrap();
        assert_eq!(c.source, "smc-system");
        assert_eq!(
            c.parse_row("0,1000,3.5,12.25").unwrap().reading,
            Reading::Watts(12.25)
        );
        let c = Columns::from_header("Delta,Time,CPU_POWER (Watts)").unwrap();
        assert_eq!(c.source, "smc-cpu");
    }

    #[test]
    fn intel_uses_package_plus_dram() {
        let c = Columns::from_header(
            "Delta,Time,CPU_USAGE_0,DRAM_ENERGY (J),PACKAGE_ENERGY (J),PP0_ENERGY (J),PP1_ENERGY (J)",
        )
        .unwrap();
        assert_eq!(c.source, "rapl-package+dram");
        let s = c.parse_row("200,5000,12.5,40.5,1000.25,700,3").unwrap();
        assert_eq!(s.t_ms, 5200);
        assert_eq!(s.reading, Reading::Joules(vec![1000.25, 40.5]));
    }

    #[test]
    fn intel_without_dram_and_amd() {
        let c = Columns::from_header("Delta,Time,PACKAGE_ENERGY (J),PP0_ENERGY (J)").unwrap();
        assert_eq!(c.source, "rapl-package");
        let c = Columns::from_header("Delta,Time,CORE0_ENERGY (J),CPU_ENERGY (J)").unwrap();
        assert_eq!(c.source, "rapl-amd-package");
        assert_eq!(
            c.parse_row("200,1,0.5,77").unwrap().reading,
            Reading::Joules(vec![77.0])
        );
    }

    #[test]
    fn headers_without_energy_are_rejected() {
        assert!(Columns::from_header("Delta,Time,CPU_USAGE_0,TOTAL_MEMORY").is_none());
        assert!(Columns::from_header("thread 'main' panicked at src/main.rs").is_none());
    }

    #[test]
    fn broken_rows_are_skipped() {
        let c = Columns::from_header("Delta,Time,SYSTEM_POWER (Watts)").unwrap();
        assert!(c.parse_row("").is_none());
        assert!(c.parse_row("200,1000").is_none());
        assert!(c.parse_row("200,1000,abc").is_none());
    }
}
