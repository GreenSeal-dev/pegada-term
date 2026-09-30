//! `pegada-term status`.

use crate::fmt::{self, Style};
use crate::state::State;
use crate::{daemon, paths, stats};

fn env_u64(name: &str) -> Option<u64> {
    paths::var(name)?.parse().ok()
}

pub fn run() -> i32 {
    let style = Style::detect();
    let row =
        |label: &str, value: String| println!("  {}  {value}", style.dim(&format!("{label:<10}")));
    println!(
        "{} {}",
        style.bolt(),
        style.bold(&format!("pegada-term {}", env!("CARGO_PKG_VERSION")))
    );

    let state = State::read(&paths::state_file());
    let fresh = state.as_ref().filter(|s| s.is_fresh(paths::now_ms()));
    match (daemon::running_pid(), fresh) {
        (Some(pid), Some(s)) => row(
            "sampler",
            format!(
                "{} running (pid {pid}, every {} ms)",
                style.ok(),
                s.interval_ms
            ),
        ),
        (Some(pid), None) => row(
            "sampler",
            format!("{} starting (pid {pid}), no samples yet", style.warn()),
        ),
        (None, _) => row(
            "sampler",
            format!("{} not running (run `pegada-term doctor`)", style.bad()),
        ),
    }
    match fresh {
        Some(s) => {
            let resolution = if s.res_ms > s.interval_ms {
                format!(", value updates every {}", fmt::duration(s.res_ms))
            } else {
                String::new()
            };
            row("sensor", format!("{}{resolution}", s.source));
            row("power now", fmt::power(s.power_mw));
            row(
                "idle",
                format!("{} (baseline while no command runs)", fmt::power(s.idle_mw)),
            );
        }
        None => {
            if let Some(reason) = std::fs::read_to_string(paths::failed_file())
                .ok()
                .and_then(|t| t.split_once(' ').map(|(_, r)| r.trim().to_string()))
            {
                row("sensor", format!("{} {reason}", style.bad()));
            }
        }
    }

    let midnight = stats::local_midnight((paths::now_ms() / 1000) as i64);
    let today: Vec<_> = stats::load()
        .into_iter()
        .filter(|r| r.epoch_s >= midnight)
        .collect();
    let total: u64 = today.iter().map(|r| r.total_mj).sum();
    let above: u64 = today.iter().map(|r| r.above_mj).sum();
    row(
        "today",
        format!(
            "{} in {} commands ({} above idle)",
            fmt::watt_hours(total),
            today.len(),
            fmt::watt_hours(above)
        ),
    );
    match env_u64("PEGADA_TERM_SESSION_MJ") {
        Some(session) => row(
            "session",
            format!(
                "{} in {} commands ({} above idle)",
                fmt::energy(session),
                env_u64("PEGADA_TERM_SESSION_CMDS").unwrap_or(0),
                fmt::energy(env_u64("PEGADA_TERM_SESSION_ABOVE_MJ").unwrap_or(0))
            ),
        ),
        None => row(
            "session",
            "unknown: this shell has no pegada-term hook".to_string(),
        ),
    }
    0
}
