//! `pegada-term doctor`: what is installed, what is wrong, and the exact fix.

use std::fs;
use std::os::unix::fs::{MetadataExt, PermissionsExt};
use std::path::Path;
use std::process::Command;
use std::thread::sleep;
use std::time::Duration;

use crate::fmt::{self, Style};
use crate::state::State;
use crate::{daemon, paths, sessions};

const INSTALL_ONE_LINER: &str =
    "curl -fsSL https://raw.githubusercontent.com/GreenSeal-dev/pegada-term/main/install.sh | sh";

pub fn output(cmd: &str, args: &[&str]) -> Option<String> {
    let out = Command::new(cmd).args(args).output().ok()?;
    out.status
        .success()
        .then(|| String::from_utf8_lossy(&out.stdout).trim().to_string())
}

#[derive(PartialEq)]
pub enum Virt {
    None,
    Wsl,
    Container,
    Vm,
}

pub fn virtualization() -> Virt {
    if cfg!(target_os = "macos") {
        return match output("sysctl", &["-n", "kern.hv_vmm_present"]).as_deref() {
            Some("1") => Virt::Vm,
            _ => Virt::None,
        };
    }
    let read = |p: &str| fs::read_to_string(p).unwrap_or_default();
    let version = read("/proc/version").to_lowercase();
    if version.contains("microsoft") || version.contains("wsl") {
        return Virt::Wsl;
    }
    let cgroup = read("/proc/1/cgroup");
    if Path::new("/.dockerenv").exists()
        || Path::new("/run/.containerenv").exists()
        || ["docker", "kubepods", "lxc"]
            .iter()
            .any(|k| cgroup.contains(k))
    {
        return Virt::Container;
    }
    if read("/proc/cpuinfo").contains(" hypervisor") {
        return Virt::Vm;
    }
    Virt::None
}

struct Report {
    style: Style,
    fixes: Vec<String>,
}

impl Report {
    fn ok(&self, label: &str, text: &str) {
        println!("  {} {label:<12} {text}", self.style.ok());
    }
    fn warn(&self, label: &str, text: &str) {
        println!("  {} {label:<12} {text}", self.style.warn());
    }
    fn bad(&mut self, label: &str, text: &str, fix: &str) {
        println!("  {} {label:<12} {text}", self.style.bad());
        if !fix.is_empty() && !self.fixes.iter().any(|f| f == fix) {
            self.fixes.push(fix.to_string());
        }
    }
}

fn check_linux_msr(r: &mut Report, energibridge: Option<&Path>) {
    const SETUP: &str = "sudo pegada-term setup";
    if !cfg!(target_arch = "x86_64") {
        r.bad(
            "rapl",
            "RAPL exists only on x86-64; EnergiBridge has no sensor for this CPU on Linux",
            "",
        );
        return;
    }
    let msr = Path::new("/dev/cpu/0/msr");
    if Path::new("/sys/module/msr").exists() || msr.exists() {
        r.ok("msr module", "loaded");
    } else {
        r.bad("msr module", "not loaded", SETUP);
    }
    match fs::metadata(msr) {
        Ok(m) => {
            let mode = m.permissions().mode() & 0o777;
            if mode & 0o040 != 0 && m.gid() != 0 {
                r.ok(
                    "msr device",
                    &format!("group-readable (gid {}, mode {mode:o})", m.gid()),
                );
            } else {
                r.bad(
                    "msr device",
                    &format!("/dev/cpu/*/msr is root-only (mode {mode:o})"),
                    SETUP,
                );
            }
        }
        Err(_) => r.bad("msr device", "/dev/cpu/0/msr does not exist", SETUP),
    }
    if let Some(path) = energibridge {
        let caps = output("getcap", &[&path.to_string_lossy()]).unwrap_or_default();
        let setgid = fs::metadata(path).is_ok_and(|m| m.permissions().mode() & 0o2000 != 0);
        if caps.contains("cap_sys_rawio") && setgid {
            r.ok(
                "privileges",
                "energibridge is setgid msr with cap_sys_rawio",
            );
        } else {
            r.bad(
                "privileges",
                "energibridge lacks cap_sys_rawio and/or setgid msr, so it cannot read MSRs",
                SETUP,
            );
        }
    }
}

fn check_sampler(r: &mut Report) {
    let state = State::read(&paths::state_file());
    let fresh = state.as_ref().filter(|s| s.is_fresh(paths::now_ms()));
    let daemon_pid = daemon::running_pid();
    match (daemon_pid, fresh) {
        (Some(pid), Some(s)) => {
            r.ok(
                "sampler",
                &format!("running, pid {pid}, every {} ms", s.interval_ms),
            );
            r.ok(
                "sensor",
                &format!(
                    "{}: {} now, {} idle, value updates every {}",
                    s.source,
                    fmt::power(s.power_mw),
                    fmt::power(s.idle_mw),
                    fmt::duration(s.res_ms)
                ),
            );
        }
        (Some(pid), None) => r.warn(
            "sampler",
            &format!("pid {pid} is up but has no fresh sample"),
        ),
        (None, _) => {
            let live = sessions::count();
            if live == 0 {
                r.warn(
                    "sampler",
                    "not running (normal: no shell with the pegada-term hook is open)",
                );
            } else {
                r.bad(
                    "sampler",
                    &format!("not running although {live} shell(s) are registered"),
                    "pegada-term restart",
                );
            }
        }
    }
    if let Some(text) = fs::read_to_string(paths::failed_file())
        .ok()
        .filter(|_| fresh.is_none())
    {
        let reason = text.split_once(' ').map_or(text.trim(), |(_, r)| r.trim());
        r.bad("last error", reason, "");
    }
    // CPU time of the two sampler processes, straight from ps.
    let pids: Vec<String> = [daemon_pid, daemon::read_pid(&paths::sensor_pid_file())]
        .into_iter()
        .flatten()
        .map(|p| p.to_string())
        .collect();
    if !pids.is_empty() {
        if let Some(ps) = output(
            "ps",
            &["-o", "pid=,time=,%cpu=,comm=", "-p", &pids.join(",")],
        ) {
            for line in ps.lines() {
                let f: Vec<&str> = line.split_whitespace().collect();
                if let [pid, time, cpu, comm, ..] = f[..] {
                    let name = comm.rsplit('/').next().unwrap_or(comm);
                    r.ok(
                        "cpu",
                        &format!("{name} (pid {pid}): {time} CPU time, {cpu}% now"),
                    );
                }
            }
        }
    }
}

pub fn run() -> i32 {
    let style = Style::detect();
    let mut r = Report {
        style,
        fixes: Vec::new(),
    };
    println!(
        "{} {}",
        style.bolt(),
        style.bold(&format!(
            "pegada-term doctor ({})",
            env!("CARGO_PKG_VERSION")
        ))
    );
    let os = std::env::consts::OS;
    r.ok("platform", &format!("{os} {}", std::env::consts::ARCH));

    let virt = virtualization();
    let linux = cfg!(target_os = "linux");
    match virt {
        Virt::None => r.ok("hardware", "bare metal (no hypervisor detected)"),
        Virt::Wsl => r.bad(
            "hardware",
            "WSL: Windows does not expose MSRs to Linux, so there is no energy sensor here",
            "",
        ),
        Virt::Container => r.bad(
            "hardware",
            "container: /dev/cpu/*/msr is normally not available inside containers",
            "",
        ),
        Virt::Vm => r.bad(
            "hardware",
            "virtual machine: energy counters are usually not passed through to guests",
            "",
        ),
    }

    let energibridge = paths::find_energibridge();
    match &energibridge {
        Some(path) => {
            let version = output(&path.to_string_lossy(), &["--version"])
                .unwrap_or_else(|| "version unknown".into());
            r.ok("energibridge", &format!("{} ({version})", path.display()));
        }
        None => r.bad(
            "energibridge",
            "not found (looked in $PEGADA_TERM_ENERGIBRIDGE, /usr/local/lib/pegada-term, ~/.local/share/pegada-term, PATH)",
            INSTALL_ONE_LINER,
        ),
    }
    if linux && virt != Virt::Wsl {
        check_linux_msr(&mut r, energibridge.as_deref());
    }

    check_sampler(&mut r);
    r.ok(
        "files",
        &format!(
            "state {} · history and log {}",
            paths::runtime_dir().display(),
            paths::state_dir().display()
        ),
    );

    let log = fs::read_to_string(paths::log_file()).unwrap_or_default();
    let tail: Vec<&str> = log.lines().rev().take(8).collect();
    if !tail.is_empty() {
        println!("\n  {}", style.dim("last log lines:"));
        for line in tail.iter().rev() {
            println!("    {}", style.dim(line));
        }
    }
    if r.fixes.is_empty() {
        return 0;
    }
    println!("\n  {}", style.bold("To fix:"));
    for fix in &r.fixes {
        println!("    {fix}");
    }
    1
}

/// One-line sensor test for the installer: `sensor OK: smc-system, 7.9 W idle`.
pub fn smoke() -> i32 {
    let session = match sessions::register_self() {
        Ok(path) => path,
        Err(e) => {
            println!("sensor unavailable: {e}");
            return 1;
        }
    };
    let _ = fs::remove_file(paths::failed_file());
    let before = State::read(&paths::state_file()).map_or(0, |s| s.seq);
    daemon::ensure_running();
    let mut result = None;
    for _ in 0..100 {
        sleep(Duration::from_millis(100));
        if paths::failed_file().exists() {
            break;
        }
        // A few samples, so the idle estimate is not a single reading.
        if let Some(s) = State::read(&paths::state_file())
            .filter(|s| s.is_fresh(paths::now_ms()) && s.seq >= before + 6 && s.power_mw > 0)
        {
            result = Some(s);
            break;
        }
    }
    let _ = fs::remove_file(session);
    match result {
        Some(s) => {
            println!("sensor OK: {}, {} idle", s.source, fmt::power(s.idle_mw));
            0
        }
        None => {
            let reason = fs::read_to_string(paths::failed_file())
                .ok()
                .and_then(|t| t.split_once(' ').map(|(_, r)| r.trim().to_string()))
                .unwrap_or_else(|| "no samples arrived".into());
            println!("sensor unavailable: {reason}");
            1
        }
    }
}
