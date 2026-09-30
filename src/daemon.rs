//! The shared sampler: one per user, started lazily, exits with the last shell.

use std::fs::{self, File, OpenOptions};
use std::io::{self, Read, Seek, Write};
use std::os::fd::AsRawFd;
use std::os::unix::process::CommandExt;
use std::process::Command;
use std::thread::sleep;
use std::time::{Duration, Instant};

use crate::engine::Engine;
use crate::paths;
use crate::sensor::{EnergiBridgeProcess, Sensor};
use crate::sessions;
use crate::sessions::BusyWatch;
use crate::state::{State, StateWriter};

pub const DEFAULT_INTERVAL_MS: u64 = 500;
/// A sensor run shorter than this counts as a failed start.
const QUICK_FAILURE: Duration = Duration::from_secs(5);
const MAX_QUICK_FAILURES: u32 = 3;
const MAX_LOG_BYTES: u64 = 256 * 1024;
/// Samples between checks that our files still exist.
const HOUSEKEEPING_EVERY: u64 = 32;

pub fn interval_ms() -> u64 {
    paths::var("PEGADA_TERM_INTERVAL")
        .and_then(|v| v.parse::<u64>().ok())
        .map_or(DEFAULT_INTERVAL_MS, |v| v.clamp(100, 10_000))
}

pub fn log(msg: &str) {
    if let Ok(mut f) = OpenOptions::new()
        .create(true)
        .append(true)
        .open(paths::log_file())
    {
        let _ = writeln!(f, "[{}] {msg}", paths::now_ms() / 1000);
    }
}

/// Pid of the running daemon, if the lock is really held.
pub fn running_pid() -> Option<i32> {
    let mut file = File::open(paths::lock_file()).ok()?;
    if try_lock(&file) {
        unsafe { libc::flock(file.as_raw_fd(), libc::LOCK_UN) };
        return None;
    }
    let mut text = String::new();
    file.read_to_string(&mut text).ok()?;
    text.trim().parse().ok()
}

fn try_lock(file: &File) -> bool {
    unsafe { libc::flock(file.as_raw_fd(), libc::LOCK_EX | libc::LOCK_NB) == 0 }
}

pub fn read_pid(path: &std::path::Path) -> Option<i32> {
    fs::read_to_string(path).ok()?.trim().parse().ok()
}

/// Starts the daemon unless the state file shows a live one.
pub fn ensure_running() {
    let fresh = State::read(&paths::state_file()).is_some_and(|s| s.is_fresh(paths::now_ms()));
    if !fresh {
        start_detached();
    }
}

/// Double fork + setsid: the daemon survives the shell and the terminal, and
/// the caller returns at once.
pub fn start_detached() {
    unsafe {
        match libc::fork() {
            -1 => {}
            0 => {
                libc::setsid();
                if libc::fork() != 0 {
                    libc::_exit(0);
                }
                detach_stdio();
                // Re-exec so `ps` shows `pegada-term daemon`, whatever command started it.
                if let Ok(exe) = std::env::current_exe() {
                    let _ = Command::new(exe).arg("daemon").exec();
                }
                libc::_exit(run());
            }
            child => {
                let mut status = 0;
                libc::waitpid(child, &mut status, 0);
            }
        }
    }
}

/// stdin/stdout to /dev/null (stdout may be the pipe of `$(pegada-term init …)`),
/// stderr to the log so panics are not lost.
unsafe fn detach_stdio() {
    let _ = paths::ensure_dirs();
    let _ = std::env::set_current_dir("/");
    if let Ok(null) = OpenOptions::new().read(true).write(true).open("/dev/null") {
        libc::dup2(null.as_raw_fd(), 0);
        libc::dup2(null.as_raw_fd(), 1);
        libc::dup2(null.as_raw_fd(), 2);
    }
    if let Ok(log) = OpenOptions::new()
        .create(true)
        .append(true)
        .open(paths::log_file())
    {
        libc::dup2(log.as_raw_fd(), 2);
    }
}

fn trim_log() {
    let path = paths::log_file();
    if fs::metadata(&path).is_ok_and(|m| m.len() > MAX_LOG_BYTES) {
        if let Ok(text) = fs::read(&path) {
            let _ = fs::write(&path, &text[text.len() - (MAX_LOG_BYTES as usize / 2)..]);
        }
    }
}

fn give_up(reason: &str) -> i32 {
    log(&format!("giving up: {reason}"));
    let _ = fs::write(
        paths::failed_file(),
        format!("{} {reason}\n", paths::now_ms()),
    );
    1
}

/// The daemon itself. Returns the process exit code.
pub fn run() -> i32 {
    if let Err(e) = paths::ensure_dirs() {
        eprintln!("pegada-term: cannot create directories: {e}");
        return 1;
    }
    let Ok(mut lock) = OpenOptions::new()
        .create(true)
        .truncate(false)
        .read(true)
        .write(true)
        .open(paths::lock_file())
    else {
        return 1;
    };
    if !try_lock(&lock) {
        return 0; // another daemon is already running
    }
    let _ = lock.set_len(0);
    let _ = lock.rewind();
    let _ = write!(lock, "{}", std::process::id());
    trim_log();
    let _ = fs::remove_file(paths::stop_file());

    let interval = interval_ms();
    let Some(energibridge) = paths::find_energibridge() else {
        return give_up("energibridge not found (run `pegada-term doctor`)");
    };
    let Ok(exe) = std::env::current_exe() else {
        return give_up("cannot find my own executable");
    };
    let watchdog = [exe.to_string_lossy().into_owned(), "__watchdog".to_string()];
    std::env::set_var("PEGADA_TERM_DAEMON_PID", std::process::id().to_string());
    log(&format!(
        "daemon {} started: {} every {interval} ms",
        std::process::id(),
        energibridge.display()
    ));

    let previous = State::read(&paths::state_file());
    let idle_seed = fs::read_to_string(paths::idle_file())
        .ok()
        .and_then(|v| v.trim().parse().ok());
    let mut engine = Engine::new(interval, previous.as_ref(), idle_seed);
    let mut writer = match StateWriter::open(&paths::state_file()) {
        Ok(writer) => writer,
        Err(e) => return give_up(&format!("cannot open the state file: {e}")),
    };
    let mut busy = BusyWatch::new();
    let mut quick_failures = 0;
    let mut lost_lock = false;

    let code = loop {
        let started = Instant::now();
        let stderr = OpenOptions::new()
            .create(true)
            .append(true)
            .open(paths::log_file())
            .or_else(|_| File::open("/dev/null"));
        let spawned = stderr.and_then(|stderr| {
            EnergiBridgeProcess::spawn(&energibridge, interval, &watchdog, stderr)
        });
        match spawned {
            Ok(mut sensor) => {
                let _ = fs::write(paths::sensor_pid_file(), sensor.pid().to_string());
                engine.rebase(sensor.source());
                let mut samples = 0u64;
                while let Ok(Some(sample)) = sensor.next_sample() {
                    let state = engine.push(&sample, busy.any_busy());
                    if let Err(e) = writer.write(state) {
                        log(&format!("cannot write state: {e}"));
                    }
                    samples += 1;
                    if samples % HOUSEKEEPING_EVERY == 0 {
                        // The runtime directory was wiped: a new daemon will be
                        // started by the next shell, so this one must go.
                        if !paths::lock_file().exists() {
                            log("lock file is gone; exiting");
                            lost_lock = true;
                            let _ = sensor.kill();
                            break;
                        }
                        let _ = writer.reopen_if_missing();
                    }
                    if samples == 4 {
                        let _ = fs::remove_file(paths::failed_file());
                    }
                }
                let status = sensor
                    .wait()
                    .map_or("unknown".to_string(), |s| s.to_string());
                log(&format!("sensor ended after {samples} samples ({status})"));
            }
            Err(e) => log(&format!("sensor failed to start: {e}")),
        }

        if fs::remove_file(paths::stop_file()).is_ok() || lost_lock {
            break 0;
        }
        if sessions::count() == 0 {
            break 0;
        }
        if started.elapsed() < QUICK_FAILURE {
            quick_failures += 1;
            if quick_failures >= MAX_QUICK_FAILURES {
                break give_up("the sensor keeps exiting right after start; see the log");
            }
        } else {
            quick_failures = 0;
        }
        sleep(Duration::from_millis(300));
    };

    if engine.idle_mw() > 0 {
        let _ = fs::write(paths::idle_file(), engine.idle_mw().to_string());
    }
    let _ = fs::remove_file(paths::sensor_pid_file());
    log("daemon exiting");
    code
}

/// Ask the sampler to exit and wait for it. Returns false if it was not running.
pub fn stop() -> io::Result<bool> {
    let Some(daemon) = running_pid() else {
        return Ok(false);
    };
    fs::write(paths::stop_file(), "")?;
    if let Some(watchdog) = read_pid(&paths::watchdog_pid_file()) {
        unsafe { libc::kill(watchdog, libc::SIGTERM) };
    }
    for _ in 0..40 {
        if running_pid().is_none() {
            break;
        }
        sleep(Duration::from_millis(100));
    }
    if running_pid().is_some() {
        // The polite way did not work; take the pieces down directly.
        for pid in [Some(daemon), read_pid(&paths::sensor_pid_file())]
            .into_iter()
            .flatten()
        {
            unsafe { libc::kill(pid, libc::SIGTERM) };
        }
    }
    let _ = fs::remove_file(paths::stop_file());
    Ok(true)
}
