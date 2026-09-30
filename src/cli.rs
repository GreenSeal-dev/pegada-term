use std::path::PathBuf;

use clap::{Parser, Subcommand, ValueEnum};

use crate::init::Shell;
use crate::{daemon, doctor, init, paths, setup, stats, status, uninstall, watch, watchdog};

#[derive(Parser)]
#[command(
    name = "pegada-term",
    version,
    about = "Shows the energy used by every command you run in the terminal"
)]
struct Cli {
    #[command(subcommand)]
    command: Option<Cmd>,
}

#[derive(Clone, Copy, ValueEnum)]
pub enum Period {
    Today,
    Week,
    All,
}

#[derive(Subcommand)]
enum Cmd {
    /// Print the shell hook: eval "$(pegada-term init zsh)"
    Init {
        shell: Shell,
        /// Do not start the sampler
        #[arg(long)]
        no_start: bool,
    },
    /// Sampler state, sensor, live power, idle baseline and totals (default)
    Status,
    /// Top commands by energy
    Stats {
        #[arg(value_enum, default_value = "today")]
        period: Period,
    },
    /// Full-screen live power meter
    Watch,
    /// Check the installation and print the fix for anything that is wrong
    Doctor {
        /// Only test the sensor and print one line (used by the installer)
        #[arg(long)]
        smoke: bool,
    },
    /// Linux: give EnergiBridge access to the RAPL counters (needs root)
    Setup {
        /// EnergiBridge binary to install
        #[arg(long)]
        energibridge: Option<PathBuf>,
    },
    /// Start the sampler
    Start,
    /// Stop the sampler
    Stop,
    /// Restart the sampler
    Restart,
    /// Show the per-command line again (needs the shell hook)
    On,
    /// Hide the per-command line in this shell (needs the shell hook)
    Off,
    /// Remove pegada-term and everything it installed
    Uninstall {
        /// Do not ask for confirmation
        #[arg(long, short)]
        yes: bool,
        /// Also delete the command history
        #[arg(long)]
        purge: bool,
    },
    /// Print the version
    Version,
    #[command(hide = true)]
    Daemon {
        /// Detach and return at once
        #[arg(long)]
        start: bool,
    },
    #[command(name = "__watchdog", hide = true)]
    Watchdog,
}

fn start() -> i32 {
    if daemon::running_pid().is_none() {
        // The sampler exits when no shell is registered, so `start` only makes
        // sense from a shell that has the hook (or while `watch` runs).
        let _ = std::fs::remove_file(paths::failed_file());
        daemon::start_detached();
    }
    0
}

pub fn run() -> i32 {
    match Cli::parse().command.unwrap_or(Cmd::Status) {
        Cmd::Init { shell, no_start } => {
            print!("{}", init::script(shell));
            if !no_start && paths::ensure_dirs().is_ok() {
                daemon::ensure_running();
            }
            0
        }
        Cmd::Status => status::run(),
        Cmd::Stats { period } => stats::run(period),
        Cmd::Watch => watch::run(),
        Cmd::Doctor { smoke } => {
            if smoke {
                doctor::smoke()
            } else {
                doctor::run()
            }
        }
        Cmd::Setup { energibridge } => setup::run(energibridge),
        Cmd::Start => start(),
        Cmd::Stop => match daemon::stop() {
            Ok(true) => 0,
            Ok(false) => {
                println!("pegada-term: the sampler is not running");
                0
            }
            Err(e) => {
                eprintln!("pegada-term: {e}");
                1
            }
        },
        Cmd::Restart => {
            let _ = daemon::stop();
            start()
        }
        Cmd::On | Cmd::Off => {
            eprintln!(
                "pegada-term: on/off are handled by the shell hook. Add this to your shell rc file:\n  eval \"$(pegada-term init zsh)\"    # or bash; for fish: pegada-term init fish | source"
            );
            1
        }
        Cmd::Uninstall { yes, purge } => uninstall::run(yes, purge),
        Cmd::Version => {
            println!("pegada-term {}", env!("CARGO_PKG_VERSION"));
            0
        }
        Cmd::Daemon { start } => {
            if start {
                daemon::start_detached();
                0
            } else {
                daemon::run()
            }
        }
        Cmd::Watchdog => watchdog::run(),
    }
}
