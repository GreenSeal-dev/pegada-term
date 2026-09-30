//! The command EnergiBridge "measures". It runs for as long as some shell is
//! registered, so EnergiBridge keeps sampling; when it exits, EnergiBridge and
//! the daemon exit too.
//!
//! It must never write to stdout: that is the daemon's CSV pipe.

use std::fs;
use std::thread::sleep;
use std::time::Duration;

use crate::daemon::read_pid;
use crate::paths;
use crate::sessions;

const CHECK_EVERY_S: u32 = 3;
/// Consecutive empty checks before exiting (about 9 s).
const EMPTY_CHECKS: u32 = 3;

pub fn run() -> i32 {
    let _ = fs::write(paths::watchdog_pid_file(), std::process::id().to_string());
    let parent = unsafe { libc::getppid() };
    let daemon: Option<i32> = paths::var("PEGADA_TERM_DAEMON_PID").and_then(|v| v.parse().ok());
    let mut empty = 0;
    let mut tick = 0;
    loop {
        sleep(Duration::from_secs(1));
        // EnergiBridge died (crash or kill -9): nobody is reading us any more.
        if unsafe { libc::getppid() } != parent {
            break;
        }
        tick += 1;
        if tick % CHECK_EVERY_S != 0 {
            continue;
        }
        if daemon.is_some_and(|pid| !sessions::pid_alive(pid)) {
            break;
        }
        if sessions::reap_and_count() == 0 {
            empty += 1;
            if empty >= EMPTY_CHECKS {
                break;
            }
        } else {
            empty = 0;
        }
    }
    // Only remove the pid file if it is still ours: a new watchdog may have replaced it.
    let mine =
        read_pid(&paths::watchdog_pid_file()).is_some_and(|p| p as u32 == std::process::id());
    if mine {
        let _ = fs::remove_file(paths::watchdog_pid_file());
    }
    0
}
