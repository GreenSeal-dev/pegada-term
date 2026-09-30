mod cli;
mod csv;
mod daemon;
mod doctor;
mod engine;
mod fmt;
mod idle;
mod init;
mod integrate;
mod paths;
mod sensor;
mod sessions;
mod setup;
mod state;
mod stats;
mod status;
mod uninstall;
mod watch;
mod watchdog;

fn main() {
    std::process::exit(cli::run());
}
