//! Test stand-in for EnergiBridge v0.0.7.
//!
//! Same CLI (`[-i interval_ms] -- <command>`) and the same CSV on stdout, but
//! power is synthetic: `IDLE_W + CORE_W x busy cores`, from the machine's real
//! CPU load. That makes `sleep` cheap and `yes` expensive without any sensor.
//!
//! `MOCK_EB_MODE` picks the platform to imitate:
//!   `rapl`  (default) Intel cumulative joules: PACKAGE/DRAM/PP0/PP1
//!   `amd`   AMD cumulative joules: CPU_ENERGY + COREn_ENERGY
//!   `watts` Apple Silicon: SYSTEM_POWER (Watts)
//!   `wrap`  like `rapl`, with the package counter a few joules from wrapping
//!   `fail`  exits with an error before printing anything, like a missing MSR
//!
//! `--burn <seconds> <threads>` is a self-terminating CPU load for the tests.
//!
//! With `MOCK_EB_LOAD_FILE` set, the load comes from that file instead of the
//! machine: `--burn` writes its thread count there while it runs, and the
//! sensor reads it as the number of busy cores. Tests on shared CI machines
//! use this, so other jobs' load does not show up in their numbers.

use std::io::Write;
use std::process::{exit, Command};
use std::thread::sleep;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

const IDLE_W: f64 = 5.0;
const CORE_W: f64 = 10.0;
/// RAPL counters are 32 bits of 61 µJ units.
const RAPL_WRAP_J: f64 = 262_144.0;
/// Share of package power attributed to DRAM in the rapl modes.
const DRAM_SHARE: f64 = 0.1;

/// (busy ticks, total ticks, cores), summed over all CPUs.
#[cfg(target_os = "macos")]
fn cpu_ticks() -> (u64, u64, f64) {
    extern "C" {
        fn mach_host_self() -> u32;
        fn host_statistics(host: u32, flavor: i32, info: *mut u32, count: *mut u32) -> i32;
    }
    const HOST_CPU_LOAD_INFO: i32 = 3;
    // user, system, idle, nice
    let mut ticks = [0u32; 4];
    let mut count = 4u32;
    unsafe {
        host_statistics(
            mach_host_self(),
            HOST_CPU_LOAD_INFO,
            ticks.as_mut_ptr(),
            &mut count,
        )
    };
    let [user, system, idle, nice] = ticks.map(u64::from);
    let cores = unsafe { libc::sysconf(libc::_SC_NPROCESSORS_ONLN) } as f64;
    (user + system + nice, user + system + nice + idle, cores)
}

#[cfg(not(target_os = "macos"))]
fn cpu_ticks() -> (u64, u64, f64) {
    let stat = std::fs::read_to_string("/proc/stat").unwrap_or_default();
    // cpu  user nice system idle iowait irq softirq steal
    let f: Vec<u64> = stat
        .lines()
        .next()
        .unwrap_or("")
        .split_whitespace()
        .skip(1)
        .filter_map(|v| v.parse().ok())
        .collect();
    let total: u64 = f.iter().take(8).sum();
    let idle = f.get(3).copied().unwrap_or(0) + f.get(4).copied().unwrap_or(0);
    let cores = unsafe { libc::sysconf(libc::_SC_NPROCESSORS_ONLN) } as f64;
    (total - idle, total, cores)
}

fn epoch_ms() -> u128 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |d| d.as_millis())
}

/// Busy cores announced by `--burn` in `MOCK_EB_LOAD_FILE`, if that is set.
fn announced_load() -> Option<f64> {
    let path = std::env::var_os("MOCK_EB_LOAD_FILE")?;
    let text = std::fs::read_to_string(path).unwrap_or_default();
    Some(text.trim().parse().unwrap_or(0.0))
}

/// Keeps `threads` cores busy for `seconds`, then stops on its own.
fn burn(seconds: f64, threads: usize) {
    let announce = std::env::var_os("MOCK_EB_LOAD_FILE");
    if let Some(path) = &announce {
        let _ = std::fs::write(path, threads.to_string());
    }
    let until = Instant::now() + Duration::from_secs_f64(seconds);
    let workers: Vec<_> = (0..threads)
        .map(|_| {
            std::thread::spawn(move || {
                let mut x = 0u64;
                while Instant::now() < until {
                    for _ in 0..10_000 {
                        x = std::hint::black_box(
                            x.wrapping_mul(6364136223846793005).wrapping_add(1),
                        );
                    }
                }
            })
        })
        .collect();
    for w in workers {
        let _ = w.join();
    }
    if let Some(path) = &announce {
        let _ = std::fs::remove_file(path);
    }
}

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let mut interval = 200u64;
    let mut command: Vec<String> = Vec::new();
    let mut i = 0;
    while i < args.len() {
        match args[i].as_str() {
            "--version" | "-V" => {
                println!("energibridge 0.0.7 (mock)");
                return;
            }
            // Test load, not an EnergiBridge option: `--burn <seconds> <threads>`.
            "--burn" => {
                let num = |k: usize| args.get(i + k).and_then(|v| v.parse::<f64>().ok());
                burn(num(1).unwrap_or(1.0), num(2).unwrap_or(1.0) as usize);
                return;
            }
            "-i" | "--interval" => {
                i += 1;
                interval = args.get(i).and_then(|v| v.parse().ok()).unwrap_or(200);
            }
            "--" => {
                command = args[i + 1..].to_vec();
                break;
            }
            _ => {}
        }
        i += 1;
    }
    let mode = std::env::var("MOCK_EB_MODE").unwrap_or_else(|_| "rapl".into());
    if mode == "fail" {
        eprintln!("thread 'main' panicked: called `Result::unwrap()` on an `Err` value: Os {{ code: 13, kind: PermissionDenied }} (mock: /dev/cpu/0/msr)");
        exit(101);
    }
    if command.is_empty() {
        eprintln!("mock-energibridge: no command given");
        exit(2);
    }
    // Like the real one: a short pause before the command starts.
    sleep(Duration::from_millis(200));
    let mut child = match Command::new(&command[0]).args(&command[1..]).spawn() {
        Ok(child) => child,
        Err(e) => {
            eprintln!("mock-energibridge: cannot run {}: {e}", command[0]);
            exit(2);
        }
    };

    let header = match mode.as_str() {
        "watts" => "Delta,Time,CPU_USAGE_0,SYSTEM_POWER (Watts),TOTAL_MEMORY,USED_MEMORY",
        "amd" => "Delta,Time,CORE0_ENERGY (J),CPU_ENERGY (J),CPU_USAGE_0,TOTAL_MEMORY,USED_MEMORY",
        _ => "Delta,Time,CPU_USAGE_0,DRAM_ENERGY (J),PACKAGE_ENERGY (J),PP0_ENERGY (J),PP1_ENERGY (J),TOTAL_MEMORY,USED_MEMORY",
    };
    let stdout = std::io::stdout();
    let mut out = stdout.lock();
    let _ = writeln!(out, "{header}");

    // Cumulative joules, as the RAPL registers would show them.
    let mut package = if mode == "wrap" {
        RAPL_WRAP_J - 12.0
    } else {
        1234.5
    };
    let mut dram = 321.0;
    let (mut busy_before, mut total_before, cores) = cpu_ticks();
    let mut watts = IDLE_W;
    let mut stamp = epoch_ms();
    let mut clock = Instant::now();
    let mut first = true;
    loop {
        let delta_ms = clock.elapsed().as_millis();
        if !first {
            let (busy, total, _) = cpu_ticks();
            if let Some(busy_cores) = announced_load() {
                watts = IDLE_W + CORE_W * busy_cores;
            } else if total > total_before {
                let load = (busy - busy_before) as f64 / (total - total_before) as f64;
                watts = IDLE_W + CORE_W * cores * load;
            }
            busy_before = busy;
            total_before = total;
            let joules = watts * delta_ms as f64 / 1000.0;
            package = (package + joules * (1.0 - DRAM_SHARE)) % RAPL_WRAP_J;
            dram += joules * DRAM_SHARE;
        }
        let row = match mode.as_str() {
            "watts" => format!("{delta_ms},{stamp},50,{watts},34359738368,21710716928"),
            "amd" => {
                let total = package + dram;
                format!(
                    "{delta_ms},{stamp},{},{total},50,34359738368,21710716928",
                    total / cores
                )
            }
            _ => format!(
                "{delta_ms},{stamp},50,{dram},{package},{},0,34359738368,21710716928",
                package * 0.8
            ),
        };
        if writeln!(out, "{row}").is_err() {
            let _ = child.kill();
            exit(1);
        }
        first = false;
        stamp = epoch_ms();
        clock = Instant::now();
        if let Ok(Some(_)) = child.try_wait() {
            break;
        }
        sleep(Duration::from_millis(interval));
    }
}
