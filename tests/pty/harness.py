"""Drives real interactive shells through a PTY, with mock-energibridge as the sensor."""

import os
import re
import shutil
import subprocess
import tempfile
import time
from pathlib import Path

import pexpect

ROOT = Path(__file__).resolve().parents[2]
TARGET = Path(os.environ.get("PEGADA_TERM_TARGET_DIR", ROOT / "target" / "release"))
BIN = TARGET / "pegada-term"
MOCK = TARGET / "mock-energibridge"
PROMPT = "PROMPT> "
ANSI = re.compile(r"\x1b\[[0-9;?<=>]*[ -/]*[@-~]|\x1b\][^\x07\x1b]*(?:\x07|\x1b\\)|\x1b[=>]|\x1b[()][0-9A-Za-z]|\x1bP.*?\x1b\\")

# bash 3.2 (macOS /bin/bash) and bash 5 take different code paths for the clock.
SHELLS = {
    "zsh": shutil.which("zsh"),
    "bash": next(
        (p for p in ("/opt/homebrew/bin/bash", "/usr/local/bin/bash", shutil.which("bash") or "") if os.path.exists(p)),
        None,
    ),
    "bash3": "/bin/bash" if os.path.exists("/bin/bash") else None,
    "fish": shutil.which("fish"),
    # zsh without zsh/system: the hook's by-path fallback.
    "zsh-nofd": shutil.which("zsh"),
}


def bash_major(path):
    out = subprocess.run([path, "-c", "echo $BASH_VERSINFO"], capture_output=True, text=True)
    return int(out.stdout.strip() or 0)


def available(name):
    path = SHELLS.get(name)
    if not path:
        return False
    if name == "bash":
        return bash_major(path) >= 5
    if name == "bash3":
        return bash_major(path) < 5
    return True


def strip_ansi(text):
    return ANSI.sub("", text).replace("\r", "")


class Sandbox:
    """Private HOME, runtime and state directories, so tests never touch the user's."""

    def __init__(self, mode="rapl", interval=200, extra_env=None, bashrc_pre=""):
        # Short path: macOS $TMPDIR is long and the daemon's lock lives below it.
        self.dir = Path(tempfile.mkdtemp(prefix="pgt-", dir="/tmp"))
        self.runtime = self.dir / "rt"
        self.state = self.dir / "state"
        self.env = {
            "HOME": str(self.dir),
            "ZDOTDIR": str(self.dir),
            "XDG_CONFIG_HOME": str(self.dir / ".config"),
            "PATH": f"{TARGET}:/usr/bin:/bin:/usr/sbin:/sbin",
            "TERM": "xterm-256color",
            "LANG": "en_US.UTF-8",
            "PEGADA_TERM_RUNTIME_DIR": str(self.runtime),
            "PEGADA_TERM_STATE_DIR": str(self.state),
            "PEGADA_TERM_ENERGIBRIDGE": str(MOCK),
            "PEGADA_TERM_INTERVAL": str(interval),
            "MOCK_EB_MODE": mode,
            # The mock reports the test's own --burn load, not the whole machine's.
            "MOCK_EB_LOAD_FILE": str(self.dir / "mock-load"),
        }
        self.env.update(extra_env or {})
        (self.dir / ".zshrc").write_text(
            f"unsetopt PROMPT_SP\nunset zle_bracketed_paste\nPS1='{PROMPT}'\n"
            'eval "$(pegada-term init zsh)"\n'
        )
        (self.dir / ".bashrc").write_text(
            f"PS1='{PROMPT}'\nbind 'set enable-bracketed-paste off' 2>/dev/null\n{bashrc_pre}\n"
            'eval "$(pegada-term init bash)"\n'
        )
        fish = self.dir / ".config" / "fish"
        fish.mkdir(parents=True)
        (fish / "config.fish").write_text(
            f"function fish_prompt; echo -n '{PROMPT}'; end\n"
            "function fish_greeting; end\n"
            "pegada-term init fish | source\n"
        )
        self.shells = []

    def shell(self, name):
        sh = Shell(self, name)
        self.shells.append(sh)
        return sh

    def pids(self, pattern):
        """Pids of this sandbox's processes whose command line matches."""
        # ww: Linux ps cuts lines at 80 columns otherwise.
        out = subprocess.run(["ps", "-axww", "-o", "pid=,command="], capture_output=True, text=True).stdout
        found = []
        for line in out.splitlines():
            pid, _, cmd = line.strip().partition(" ")
            if pattern in cmd and ("pegada-term" in cmd or "mock-energibridge" in cmd):
                if self.owns(int(pid)):
                    found.append(int(pid))
        return found

    def owns(self, pid):
        try:
            return str(self.runtime).encode() in Path(f"/proc/{pid}/environ").read_bytes()
        except OSError:
            pass  # no /proc (macOS), or the process is gone
        env = subprocess.run(["ps", "eww", "-o", "command=", "-p", str(pid)], capture_output=True, text=True).stdout
        return str(self.runtime) in env

    def daemons(self):
        return self.pids("pegada-term daemon")

    def sensors(self):
        return self.pids("mock-energibridge -i")

    def describe(self):
        """What is left of this sandbox, for assertion messages."""
        ps = subprocess.run(["ps", "-axww", "-o", "pid=,ppid=,stat=,command="], capture_output=True, text=True).stdout
        mine = [l for l in ps.splitlines() if "pegada-term" in l or "mock-energibridge" in l or "sh" in l.split()[-1:]]
        sessions = self.runtime / "sessions"
        files = sorted(f"{p.name}:{p.stat().st_size}" for p in sessions.iterdir()) if sessions.is_dir() else []
        return "sessions: %s\n%s" % (files, "\n".join(mine))

    def read_state(self):
        try:
            return (self.runtime / "state").read_text().split("\n")[0].split(" ")
        except OSError:
            return None

    def wait_for(self, predicate, timeout=20.0, step=0.2):
        # Monotonic: the wall clock of a VM can jump.
        deadline = time.monotonic() + timeout
        while time.monotonic() < deadline:
            if predicate():
                return True
            time.sleep(step)
        return False

    def wait_fresh(self, timeout=10.0):
        def fresh():
            state = self.read_state()
            return bool(state) and len(state) > 6 and time.time() * 1000 - int(state[1]) < 1500

        assert self.wait_for(fresh, timeout), "the sampler never produced a fresh state"

    def history(self):
        path = self.state / "history.tsv"
        if not path.exists():
            return []
        return [line.split("\t") for line in path.read_text().splitlines()]

    def close(self):
        for sh in self.shells:
            sh.close()
        subprocess.run([str(BIN), "stop"], env=self.env, capture_output=True)
        for pid in self.daemons() + self.sensors() + self.pids("__watchdog"):
            try:
                os.kill(pid, 9)
            except OSError:
                pass
        shutil.rmtree(self.dir, ignore_errors=True)


class Shell:
    def __init__(self, sandbox, name):
        self.name = name
        self.kind = "bash" if name.startswith("bash") else name.split("-")[0]
        env = dict(sandbox.env, PEGADA_TERM_NO_FD="1") if name.endswith("-nofd") else sandbox.env
        path = SHELLS[name]
        args = {
            # -d: no global rc files. Ubuntu's /etc/zsh/zshrc runs compinit, which stops to ask
            # about insecure directories.
            "zsh": ["-d", "-i"],
            "bash": ["--noprofile", "--rcfile", str(sandbox.dir / ".bashrc"), "-i"],
            "fish": ["-i"],
        }[self.kind]
        self.child = pexpect.spawn(
            path, args, env=env, encoding="utf-8", timeout=30, dimensions=(24, 160), cwd=str(sandbox.dir)
        )
        self.child.expect_exact(PROMPT)

    def run(self, command, timeout=30):
        """Runs a command and returns everything printed up to the next prompt, without ANSI codes."""
        self.child.sendline(command)
        self.child.expect_exact(PROMPT, timeout=timeout)
        out = strip_ansi(self.child.before)
        # fish repaints the prompt line while typing; keep what follows the echoed command.
        if command and command in out:
            out = out.split(command, 1)[1]
        return out.lstrip("\n")

    def run_raw(self, command, timeout=30):
        """Like run, but with the escape sequences left in."""
        self.child.sendline(command)
        self.child.expect_exact(PROMPT, timeout=timeout)
        return self.child.before

    def setenv(self, name, value):
        self.run(f"set -gx {name} {value}" if self.kind == "fish" else f"export {name}={value}")

    def status_var(self):
        return "$status" if self.kind == "fish" else "$?"

    def energy_line(self, command, timeout=30):
        """The pegada-term line printed after a command, or None."""
        # The line follows the "spaces + \r" padding, which strip_ansi leaves as spaces.
        lines = [l.strip() for l in self.run(command, timeout).splitlines()]
        lines = [l for l in lines if l.startswith(("⚡", "* "))]
        return lines[-1] if lines else None

    def close(self):
        if self.child.isalive():
            self.child.sendline("exit")
            try:
                self.child.expect(pexpect.EOF, timeout=5)
            except pexpect.ExceptionPexpect:
                pass
        self.child.close(force=True)


UNITS = {"mJ": 1e-3, "J": 1.0, "kJ": 1e3, "Wh": 3600.0, "kWh": 3.6e6}


def joules(line, label):
    """Value in joules of the number before `label` ("total" or "above idle")."""
    m = re.search(r"(~?)([\d.]+) (mJ|J|kJ|Wh|kWh) " + label, line)
    assert m, f"no '{label}' value in: {line!r}"
    return float(m.group(2)) * UNITS[m.group(3)]
