//! `pegada-term setup`: the privileged part of a Linux install.
//!
//! EnergiBridge reads RAPL through `/dev/cpu/N/msr`, which needs the `msr`
//! module, read permission on the devices and CAP_SYS_RAWIO. We give exactly
//! that to one root-owned copy of EnergiBridge, and to nothing else.

use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

use crate::doctor::{self, Virt};
use crate::paths;

pub const UDEV_RULE_FILE: &str = "/etc/udev/rules.d/70-pegada-term-msr.rules";
pub const MODULES_LOAD_FILE: &str = "/etc/modules-load.d/pegada-term-msr.conf";
const UDEV_RULE: &str = "SUBSYSTEM==\"msr\", KERNEL==\"msr[0-9]*\", GROUP=\"msr\", MODE=\"0640\"\n";

pub const TRADE_OFF: &str = "\
Security trade-off: RAPL energy counters can be used as a power side channel
(the PLATYPUS attack, CVE-2020-8694), which is why Linux restricts them to root.
This setup lets any local user read them *through EnergiBridge only*: the
binary is root-owned, setgid `msr`, and carries CAP_SYS_RAWIO. Programs it
starts inherit the group but not the capability, so they cannot open the MSR
devices. On a shared or multi-tenant machine, do not run this setup.";

fn sh(cmd: &str, args: &[&str]) -> bool {
    println!("  $ {cmd} {}", args.join(" "));
    match Command::new(cmd).args(args).status() {
        Ok(status) if status.success() => true,
        Ok(status) => {
            eprintln!("    failed ({status})");
            false
        }
        Err(e) => {
            eprintln!("    failed: {e}");
            false
        }
    }
}

fn write(path: &str, contents: &str) -> bool {
    println!("  $ write {path}");
    if let Some(dir) = Path::new(path).parent() {
        let _ = fs::create_dir_all(dir);
    }
    fs::write(path, contents)
        .map_err(|e| eprintln!("    failed: {e}"))
        .is_ok()
}

pub fn run(energibridge: Option<PathBuf>) -> i32 {
    if !cfg!(target_os = "linux") {
        return match paths::find_energibridge() {
            Some(path) => {
                println!(
                    "macOS needs no privileged setup: EnergiBridge reads the SMC without root.\nEnergiBridge: {}",
                    path.display()
                );
                0
            }
            None => {
                println!("EnergiBridge is not installed. Run `pegada-term doctor` for the fix.");
                1
            }
        };
    }
    match doctor::virtualization() {
        Virt::Wsl => {
            eprintln!("WSL does not expose MSRs, so RAPL cannot be read here. Nothing to set up.");
            return 1;
        }
        Virt::Container => {
            eprintln!("This is a container; MSR access has to be set up on the host.");
            return 1;
        }
        Virt::Vm => eprintln!("warning: this looks like a VM; RAPL is often missing in guests."),
        Virt::None => {}
    }
    let system_copy = Path::new(paths::SYSTEM_LIB_DIR).join("energibridge");
    let Some(source) = energibridge.or_else(paths::find_energibridge) else {
        eprintln!(
            "No EnergiBridge binary found. Pass one: sudo pegada-term setup --energibridge <path>"
        );
        return 1;
    };
    if unsafe { libc::geteuid() } != 0 {
        eprintln!(
            "This needs root. Run:\n  sudo {} setup --energibridge {}",
            std::env::current_exe().map_or("pegada-term".into(), |p| p.display().to_string()),
            source.display()
        );
        return 1;
    }
    println!("{TRADE_OFF}\n");

    let dst = system_copy.to_string_lossy().into_owned();
    let src = source.to_string_lossy().into_owned();
    let mut ok = sh("groupadd", &["-f", "--system", "msr"]);
    ok &= sh("mkdir", &["-p", paths::SYSTEM_LIB_DIR]);
    if source != system_copy {
        ok &= sh(
            "install",
            &["-o", "root", "-g", "msr", "-m", "2755", &src, &dst],
        );
    } else {
        ok &= sh("chown", &["root:msr", &dst]) && sh("chmod", &["2755", &dst]);
    }
    // After install: copying a file drops its capabilities.
    if !sh("setcap", &["cap_sys_rawio=ep", &dst]) {
        eprintln!("    `setcap` comes with libcap (Debian/Ubuntu: libcap2-bin, Fedora: libcap).");
        ok = false;
    }
    ok &= write(UDEV_RULE_FILE, UDEV_RULE);
    ok &= write(MODULES_LOAD_FILE, "msr\n");
    ok &= sh("modprobe", &["msr"]);
    // Apply the udev rule's permissions now, without waiting for a reboot.
    let _ = sh("udevadm", &["control", "--reload-rules"]);
    let mut devices = 0;
    for cpu in fs::read_dir("/dev/cpu").into_iter().flatten().flatten() {
        let msr = cpu.path().join("msr");
        if msr.exists() {
            let msr = msr.to_string_lossy().into_owned();
            let quiet = |cmd: &str, arg: &str| {
                Command::new(cmd)
                    .args([arg, &msr])
                    .status()
                    .is_ok_and(|s| s.success())
            };
            if quiet("chgrp", "msr") && quiet("chmod", "0640") {
                devices += 1;
            }
        }
    }
    println!("  $ chgrp msr, chmod 0640 on {devices} /dev/cpu/*/msr devices");
    if devices == 0 {
        eprintln!("    no MSR devices found: this CPU or kernel has no MSR interface");
        ok = false;
    }
    if ok {
        println!("\nDone. No re-login needed. Check with: pegada-term doctor");
        0
    } else {
        eprintln!("\nSetup did not complete; see the failed steps above.");
        1
    }
}
