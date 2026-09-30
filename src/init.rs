//! `pegada-term init <shell>`: prints the hook code, with paths filled in.

use clap::ValueEnum;

use crate::paths;

#[derive(Clone, Copy, ValueEnum)]
pub enum Shell {
    Zsh,
    Bash,
    Fish,
}

const ZSH: &str = include_str!("../hooks/pegada-term.zsh");
const BASH: &str = include_str!("../hooks/pegada-term.bash");
const FISH: &str = include_str!("../hooks/pegada-term.fish");

/// The templates hold each placeholder inside single quotes.
fn quote(shell: Shell, value: &str) -> String {
    match shell {
        Shell::Zsh | Shell::Bash => value.replace('\'', r"'\''"),
        Shell::Fish => value.replace('\\', r"\\").replace('\'', r"\'"),
    }
}

pub fn script(shell: Shell) -> String {
    let template = match shell {
        Shell::Zsh => ZSH,
        Shell::Bash => BASH,
        Shell::Fish => FISH,
    };
    let exe = std::env::current_exe()
        .map(|p| p.to_string_lossy().into_owned())
        .unwrap_or_else(|_| "pegada-term".into());
    template
        .replace("@BIN@", &quote(shell, &exe))
        .replace(
            "@RUNTIME@",
            &quote(shell, &paths::runtime_dir().to_string_lossy()),
        )
        .replace(
            "@STATE@",
            &quote(shell, &paths::state_dir().to_string_lossy()),
        )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn no_placeholder_is_left_behind() {
        for shell in [Shell::Zsh, Shell::Bash, Shell::Fish] {
            let s = script(shell);
            assert!(!s.contains("@BIN@") && !s.contains("@RUNTIME@") && !s.contains("@STATE@"));
        }
    }

    #[test]
    fn quotes_are_escaped() {
        assert_eq!(quote(Shell::Zsh, "/a'b"), r"/a'\''b");
        assert_eq!(quote(Shell::Fish, r"/a'b\c"), r"/a\'b\\c");
    }
}
