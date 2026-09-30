//! Where things live. The hooks get these paths baked in by `init`.

use std::env;
use std::fs;
use std::io;
use std::os::unix::fs::{DirBuilderExt, PermissionsExt};
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

pub const SYSTEM_LIB_DIR: &str = "/usr/local/lib/pegada-term";

pub fn var(name: &str) -> Option<String> {
    env::var(name).ok().filter(|v| !v.is_empty())
}

pub fn home() -> PathBuf {
    PathBuf::from(var("HOME").unwrap_or_else(|| "/".into()))
}

/// RAM-backed (Linux) or per-user temporary (macOS) directory for the state file.
pub fn runtime_dir() -> PathBuf {
    if let Some(dir) = var("PEGADA_TERM_RUNTIME_DIR") {
        return dir.into();
    }
    let uid = unsafe { libc::getuid() };
    if cfg!(target_os = "macos") {
        let tmp = var("TMPDIR").unwrap_or_else(|| "/tmp".into());
        return Path::new(&tmp).join(format!("pegada-term-{uid}"));
    }
    match var("XDG_RUNTIME_DIR") {
        Some(dir) => Path::new(&dir).join("pegada-term"),
        None => PathBuf::from(format!("/tmp/pegada-term-{uid}")),
    }
}

/// History and logs.
pub fn state_dir() -> PathBuf {
    if let Some(dir) = var("PEGADA_TERM_STATE_DIR") {
        return dir.into();
    }
    match var("XDG_STATE_HOME") {
        Some(dir) => Path::new(&dir).join("pegada-term"),
        None => home().join(".local/state/pegada-term"),
    }
}

/// Where the installer puts the EnergiBridge it downloads.
pub fn data_dir() -> PathBuf {
    match var("XDG_DATA_HOME") {
        Some(dir) => Path::new(&dir).join("pegada-term"),
        None => home().join(".local/share/pegada-term"),
    }
}

pub fn state_file() -> PathBuf {
    runtime_dir().join("state")
}
pub fn sessions_dir() -> PathBuf {
    runtime_dir().join("sessions")
}
pub fn lock_file() -> PathBuf {
    runtime_dir().join("daemon.lock")
}
pub fn watchdog_pid_file() -> PathBuf {
    runtime_dir().join("watchdog.pid")
}
pub fn sensor_pid_file() -> PathBuf {
    runtime_dir().join("sensor.pid")
}
/// Written by `stop` so the daemon does not restart the sensor.
pub fn stop_file() -> PathBuf {
    runtime_dir().join("stop")
}
/// Written when the daemon gives up; holds the reason.
pub fn failed_file() -> PathBuf {
    runtime_dir().join("failed")
}
pub fn history_file() -> PathBuf {
    state_dir().join("history.tsv")
}
pub fn log_file() -> PathBuf {
    state_dir().join("daemon.log")
}
pub fn idle_file() -> PathBuf {
    state_dir().join("idle_mw")
}

pub fn ensure_dirs() -> io::Result<()> {
    for dir in [runtime_dir(), sessions_dir()] {
        fs::DirBuilder::new()
            .recursive(true)
            .mode(0o700)
            .create(&dir)?;
    }
    fs::create_dir_all(state_dir())
}

fn is_executable(path: &Path) -> bool {
    fs::metadata(path).is_ok_and(|m| m.is_file() && m.permissions().mode() & 0o111 != 0)
}

pub fn find_in_path(name: &str) -> Option<PathBuf> {
    env::split_paths(&var("PATH")?)
        .map(|dir| dir.join(name))
        .find(|p| is_executable(p))
}

/// `$PEGADA_TERM_ENERGIBRIDGE`, then the system copy made by `setup`, then
/// the installer's copy, then PATH.
pub fn find_energibridge() -> Option<PathBuf> {
    if let Some(path) = var("PEGADA_TERM_ENERGIBRIDGE") {
        let path = PathBuf::from(path);
        return is_executable(&path).then_some(path);
    }
    [
        Path::new(SYSTEM_LIB_DIR).join("energibridge"),
        data_dir().join("energibridge"),
    ]
    .into_iter()
    .find(|p| is_executable(p))
    .or_else(|| find_in_path("energibridge"))
}

pub fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |d| d.as_millis() as u64)
}
