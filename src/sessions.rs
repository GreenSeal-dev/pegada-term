//! Shell sessions register by creating `sessions/<pid>`.
//!
//! A hook appends one byte when a command starts and one when it ends, so an
//! odd file length means "running a command". Appending is the cheapest thing
//! a shell can do to a file without forking, and zsh does it on a descriptor
//! it keeps open.

use std::fs;
use std::io;
use std::path::PathBuf;
use std::time::SystemTime;

use crate::paths;

pub fn pid_alive(pid: i32) -> bool {
    if pid <= 0 {
        return false;
    }
    // EPERM means the process exists but belongs to someone else.
    unsafe { libc::kill(pid, 0) == 0 || *errno() == libc::EPERM }
}

#[cfg(target_os = "macos")]
unsafe fn errno() -> *mut i32 {
    libc::__error()
}
#[cfg(not(target_os = "macos"))]
unsafe fn errno() -> *mut i32 {
    libc::__errno_location()
}

fn entries() -> Vec<(PathBuf, Option<i32>, u64)> {
    let Ok(dir) = fs::read_dir(paths::sessions_dir()) else {
        return Vec::new();
    };
    dir.flatten()
        .map(|e| {
            let pid = e.file_name().to_str().and_then(|n| n.parse().ok());
            let len = e.metadata().map_or(0, |m| m.len());
            (e.path(), pid, len)
        })
        .collect()
}

/// Number of live sessions. Files of dead shells are deleted.
pub fn reap_and_count() -> usize {
    let mut live = 0;
    for (path, pid, _) in entries() {
        if pid.is_some_and(pid_alive) {
            live += 1;
        } else {
            let _ = fs::remove_file(path);
        }
    }
    live
}

pub fn count() -> usize {
    entries()
        .iter()
        .filter(|(_, pid, _)| pid.is_some_and(pid_alive))
        .count()
}

/// Answers "is any registered shell running a command?" once per sample,
/// with one `stat` for the directory and one per session.
pub struct BusyWatch {
    dir: PathBuf,
    dir_mtime: Option<SystemTime>,
    files: Vec<PathBuf>,
}

impl BusyWatch {
    pub fn new() -> Self {
        Self {
            dir: paths::sessions_dir(),
            dir_mtime: None,
            files: Vec::new(),
        }
    }

    pub fn any_busy(&mut self) -> bool {
        // Shells come and go rarely: list the directory only when it changed.
        let mtime = fs::metadata(&self.dir).and_then(|m| m.modified()).ok();
        if mtime != self.dir_mtime || mtime.is_none() {
            self.dir_mtime = mtime;
            self.files = entries().into_iter().map(|(path, _, _)| path).collect();
        }
        self.files
            .iter()
            .any(|f| fs::metadata(f).is_ok_and(|m| m.len() % 2 == 1))
    }
}

/// Keeps the sampler alive for as long as this process runs (`watch`, `doctor`).
pub fn register_self() -> io::Result<PathBuf> {
    paths::ensure_dirs()?;
    let path = paths::sessions_dir().join(std::process::id().to_string());
    fs::write(&path, "")?;
    Ok(path)
}
