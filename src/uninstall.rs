//! `pegada-term uninstall`: reverse everything the installer did.

use std::fs;
use std::io::{BufRead, BufReader, Write};
use std::path::{Path, PathBuf};
use std::process::Command;

use crate::setup::{MODULES_LOAD_FILE, UDEV_RULE_FILE};
use crate::{daemon, paths};

const BEGIN: &str = "\n# >>> pegada-term >>>\n";
const END: &str = "# <<< pegada-term <<<\n";

/// Removes the installer's block, leaving the file as it was before.
pub fn strip_block(text: &str) -> Option<String> {
    let start = text.find(BEGIN)?;
    let end = start + text[start..].find(END)? + END.len();
    Some(format!("{}{}", &text[..start], &text[end..]))
}

fn ask(question: &str) -> bool {
    let Ok(tty) = fs::OpenOptions::new()
        .read(true)
        .write(true)
        .open("/dev/tty")
    else {
        return false;
    };
    let mut out = &tty;
    let _ = write!(out, "{question} [y/N] ");
    let mut answer = String::new();
    let _ = BufReader::new(&tty).read_line(&mut answer);
    matches!(answer.trim(), "y" | "Y" | "yes")
}

fn rc_files() -> Vec<PathBuf> {
    let home = paths::home();
    let zdotdir = paths::var("ZDOTDIR").map_or(home.clone(), PathBuf::from);
    vec![
        zdotdir.join(".zshrc"),
        home.join(".bashrc"),
        home.join(".bash_profile"),
    ]
}

fn fish_conf() -> PathBuf {
    let config = paths::var("XDG_CONFIG_HOME").map_or(paths::home().join(".config"), PathBuf::from);
    config.join("fish/conf.d/pegada-term.fish")
}

pub fn run(yes: bool, purge: bool) -> i32 {
    if !yes && !ask("Remove pegada-term, its shell hooks and its EnergiBridge copy?") {
        println!("Nothing removed.");
        return 1;
    }
    let _ = daemon::stop();

    for rc in rc_files() {
        if let Some(clean) = fs::read_to_string(&rc)
            .ok()
            .as_deref()
            .and_then(strip_block)
        {
            match fs::write(&rc, clean) {
                Ok(()) => println!("removed hook from {}", rc.display()),
                Err(e) => eprintln!("could not edit {}: {e}", rc.display()),
            }
        }
    }
    let removed = |path: &Path| {
        let gone = if path.is_dir() {
            fs::remove_dir_all(path)
        } else {
            fs::remove_file(path)
        };
        if gone.is_ok() {
            println!("removed {}", path.display());
        }
    };
    removed(&fish_conf());
    removed(&paths::data_dir());
    removed(&paths::runtime_dir());
    if purge {
        removed(&paths::state_dir());
    } else {
        removed(&paths::log_file());
        removed(&paths::idle_file());
        if paths::history_file().exists() {
            println!(
                "kept your history: {} (use --purge to delete it)",
                paths::history_file().display()
            );
        }
    }

    let system: Vec<&str> = [UDEV_RULE_FILE, MODULES_LOAD_FILE, paths::SYSTEM_LIB_DIR]
        .into_iter()
        .filter(|p| Path::new(p).exists())
        .collect();
    if !system.is_empty() {
        println!("\nSystem files from `pegada-term setup`:");
        for path in &system {
            println!("  {path}");
        }
        if yes || ask("Remove them with sudo?") {
            let mut args = vec!["rm", "-rf", "--"];
            args.extend(&system);
            let done = Command::new("sudo")
                .args(&args)
                .status()
                .is_ok_and(|s| s.success());
            if done {
                println!("removed. The `msr` group and the loaded msr module were left alone;\nMSR device permissions reset at the next boot.");
            } else {
                eprintln!("sudo failed; remove them by hand: sudo {}", args.join(" "));
            }
        } else {
            println!(
                "left in place. To remove: sudo rm -rf -- {}",
                system.join(" ")
            );
        }
    }

    match std::env::current_exe() {
        Ok(exe) if exe.starts_with(paths::home().join(".cargo")) => {
            println!(
                "\nThe binary was installed by cargo. Finish with: cargo uninstall pegada-term"
            );
        }
        Ok(exe) if exe.starts_with(paths::home().join(".local/bin")) => removed(&exe),
        Ok(exe) => println!(
            "\nNot installed by install.sh, so left in place: {}",
            exe.display()
        ),
        Err(_) => {}
    }
    println!("\npegada-term is uninstalled. Shells that are already open keep the hook until you close them.");
    0
}

#[cfg(test)]
mod tests {
    use super::*;

    const BLOCK: &str =
        "\n# >>> pegada-term >>>\neval \"$(pegada-term init zsh)\"\n# <<< pegada-term <<<\n";

    #[test]
    fn strip_restores_the_original_bytes() {
        for original in ["", "export A=1\n", "export A=1", "a\n\n\nb\n"] {
            let installed = format!("{original}{BLOCK}");
            assert_eq!(strip_block(&installed).as_deref(), Some(original));
        }
    }

    #[test]
    fn text_after_the_block_is_kept() {
        let text = format!("before\n{BLOCK}after\n");
        assert_eq!(strip_block(&text).as_deref(), Some("before\nafter\n"));
    }

    #[test]
    fn files_without_a_block_are_left_alone() {
        assert_eq!(strip_block("export A=1\n"), None);
        assert_eq!(strip_block("\n# >>> pegada-term >>>\nunterminated"), None);
    }
}
