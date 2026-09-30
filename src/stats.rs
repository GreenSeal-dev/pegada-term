//! `pegada-term stats`: top commands by energy, from `history.tsv`.

use std::collections::HashMap;
use std::fs;

use crate::cli::Period;
use crate::fmt::{self, Style, AMBER, GREEN};
use crate::paths;

/// A round default, not any particular grid; PEGADA_TERM_CARBON_INTENSITY overrides it.
const DEFAULT_CARBON_G_PER_KWH: f64 = 250.0;
/// A typical phone battery, for the "fun equivalent".
const PHONE_CHARGE_WH: f64 = 15.0;
const TOP: usize = 10;
const BAR_WIDTH: usize = 24;

pub struct Record {
    pub epoch_s: i64,
    pub dur_ms: u64,
    pub total_mj: u64,
    pub above_mj: u64,
    pub cmd: String,
}

/// `epoch  dur_ms  total_mJ  above_idle_mJ  exit  cmd_head`, tab-separated.
pub fn parse_line(line: &str) -> Option<Record> {
    let mut f = line.split('\t');
    let epoch_s = f.next()?.parse().ok()?;
    let dur_ms = f.next()?.parse().ok()?;
    let total_mj = f.next()?.parse().ok()?;
    let above_mj = f.next()?.parse().ok()?;
    let _exit = f.next()?;
    let cmd = f.next()?.trim().to_string();
    Some(Record {
        epoch_s,
        dur_ms,
        total_mj,
        above_mj,
        cmd,
    })
}

pub fn load() -> Vec<Record> {
    fs::read_to_string(paths::history_file())
        .unwrap_or_default()
        .lines()
        .filter_map(parse_line)
        .collect()
}

pub fn local_midnight(now_s: i64) -> i64 {
    unsafe {
        let mut tm: libc::tm = std::mem::zeroed();
        // `as _`: time_t's width differs between targets.
        libc::localtime_r(&(now_s as _), &mut tm);
        tm.tm_hour = 0;
        tm.tm_min = 0;
        tm.tm_sec = 0;
        libc::mktime(&mut tm) as i64
    }
}

#[derive(Default, Debug, PartialEq)]
pub struct Group {
    pub cmd: String,
    pub runs: u64,
    pub dur_ms: u64,
    pub total_mj: u64,
    pub above_mj: u64,
}

/// Groups by command, highest total energy first.
pub fn group(records: &[Record]) -> Vec<Group> {
    let mut by_cmd: HashMap<&str, Group> = HashMap::new();
    for r in records {
        let g = by_cmd.entry(&r.cmd).or_default();
        g.runs += 1;
        g.dur_ms += r.dur_ms;
        g.total_mj += r.total_mj;
        g.above_mj += r.above_mj;
    }
    let mut groups: Vec<Group> = by_cmd
        .into_iter()
        .map(|(cmd, g)| Group {
            cmd: cmd.to_string(),
            ..g
        })
        .collect();
    groups.sort_by(|a, b| b.total_mj.cmp(&a.total_mj).then(a.cmd.cmp(&b.cmd)));
    groups
}

pub fn carbon_intensity() -> (f64, bool) {
    match paths::var("PEGADA_TERM_CARBON_INTENSITY").and_then(|v| v.parse::<f64>().ok()) {
        Some(v) if v >= 0.0 => (v, false),
        _ => (DEFAULT_CARBON_G_PER_KWH, true),
    }
}

/// Enough decimals that a small value does not print as 0.00.
fn small(v: f64) -> String {
    if v >= 10.0 {
        format!("{v:.0}")
    } else if v >= 0.1 {
        format!("{v:.2}")
    } else {
        format!("{v:.4}")
    }
}

pub fn run(period: Period) -> i32 {
    let style = Style::detect();
    let now = (paths::now_ms() / 1000) as i64;
    let (since, label) = match period {
        Period::Today => (local_midnight(now), "today"),
        Period::Week => (now - 7 * 86_400, "the last 7 days"),
        Period::All => (i64::MIN, "all time"),
    };
    let records: Vec<Record> = load().into_iter().filter(|r| r.epoch_s >= since).collect();
    if records.is_empty() {
        println!("{} No measured commands for {label} yet.", style.bolt());
        if paths::var("PEGADA_TERM_HISTORY").as_deref() == Some("0") {
            println!("  History is off (PEGADA_TERM_HISTORY=0).");
        }
        return 0;
    }
    let groups = group(&records);
    let total: u64 = groups.iter().map(|g| g.total_mj).sum();
    let above: u64 = groups.iter().map(|g| g.above_mj).sum();
    let max = groups[0].total_mj.max(1);

    println!(
        "{} {}",
        style.bolt(),
        style.bold(&format!("Top commands by energy, {label}"))
    );
    println!();
    let width = groups
        .iter()
        .take(TOP)
        .map(|g| g.cmd.chars().count())
        .max()
        .unwrap_or(0)
        .clamp(7, 28);
    println!(
        "{}",
        style.dim(&format!(
            "  {:<width$}  {:>5}  {:>9}  {:>10}  {:>9}",
            "command", "runs", "total", "above idle", "time"
        ))
    );
    for g in groups.iter().take(TOP) {
        let lit = (g.total_mj as u128 * BAR_WIDTH as u128 / max as u128) as usize;
        let lit_above = (g.above_mj as u128 * BAR_WIDTH as u128 / max as u128) as usize;
        let lit = lit.max(1);
        let (hot, _) = style.bar(lit_above.min(lit), BAR_WIDTH);
        let (rest, empty) = style.bar(lit - lit_above.min(lit), BAR_WIDTH - lit_above.min(lit));
        let cmd: String = g.cmd.chars().take(width).collect();
        println!(
            "  {:<width$}  {:>5}  {:>9}  {:>10}  {:>9}  {}{}{}",
            cmd,
            g.runs,
            fmt::energy(g.total_mj),
            fmt::energy(g.above_mj),
            fmt::duration(g.dur_ms),
            style.fg(AMBER, &hot),
            style.fg(GREEN, &rest),
            style.dim(&empty),
        );
    }
    if groups.len() > TOP {
        println!(
            "{}",
            style.dim(&format!("  … and {} more", groups.len() - TOP))
        );
    }
    let (hot, _) = style.bar(1, 1);
    println!(
        "{}",
        style.dim(&format!(
            "  bars: {hot} above idle (amber) + idle share (green) = total"
        ))
    );

    let wh = total as f64 / 3_600_000.0;
    let (intensity, is_default) = carbon_intensity();
    let grams = wh / 1000.0 * intensity;
    println!();
    println!(
        "  {} in {} commands, {} of it above idle",
        style.bold(&fmt::watt_hours(total)),
        records.len(),
        fmt::watt_hours(above)
    );
    println!(
        "  ≈ {} gCO₂ at {intensity:.0} gCO₂/kWh{}",
        small(grams),
        if is_default {
            " (default; set PEGADA_TERM_CARBON_INTENSITY for your grid)"
        } else {
            ""
        }
    );
    println!(
        "  ≈ {} phone charges ({PHONE_CHARGE_WH:.0} Wh each)",
        small(wh / PHONE_CHARGE_WH)
    );
    println!(
        "{}",
        style.dim(
            "  Whole-machine energy while each command ran; commands under the sensor's\n  resolution are not recorded. See \"What is measured\" in the README."
        )
    );
    0
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_history_lines() {
        let r = parse_line("1790762787\t12400\t142000\t38000\t0\tcargo build").unwrap();
        assert_eq!(
            (r.epoch_s, r.dur_ms, r.total_mj, r.above_mj, r.cmd.as_str()),
            (1_790_762_787, 12_400, 142_000, 38_000, "cargo build")
        );
        assert!(parse_line("garbage").is_none());
        assert!(parse_line("1\t2\t3").is_none());
    }

    #[test]
    fn groups_and_sorts_by_total() {
        let recs: Vec<Record> = [
            "1\t1000\t5000\t1000\t0\tgit push",
            "2\t2000\t90000\t60000\t0\tcargo build",
            "3\t1000\t7000\t500\t1\tgit push",
        ]
        .iter()
        .filter_map(|l| parse_line(l))
        .collect();
        let g = group(&recs);
        assert_eq!(g[0].cmd, "cargo build");
        assert_eq!(
            g[1],
            Group {
                cmd: "git push".into(),
                runs: 2,
                dur_ms: 2000,
                total_mj: 12_000,
                above_mj: 1_500
            }
        );
    }

    #[test]
    fn local_midnight_is_within_the_last_day() {
        let now = 1_790_762_787;
        let m = local_midnight(now);
        assert!(m <= now && now - m < 86_400 + 3_600);
    }
}
