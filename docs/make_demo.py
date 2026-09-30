#!/usr/bin/env python3
"""Records docs/demo.cast (asciicast v2) from a real zsh session and renders docs/demo.gif.

    cargo build --release --workspace
    PEGADA_TERM_ENERGIBRIDGE=/path/to/energibridge python3 docs/make_demo.py

Without PEGADA_TERM_ENERGIBRIDGE the mock sensor is used. Needs pexpect, and `agg`
for the GIF. The session runs in a throwaway HOME; the numbers are whatever the
sensor reported while recording.
"""

import json
import os
import shutil
import subprocess
import sys
import tempfile
import time
from pathlib import Path

import pexpect

ROOT = Path(__file__).resolve().parents[1]
COLS, ROWS = 96, 24
# What zsh prints for PS1='%F{71}❯%f '.
PROMPT = "❯\x1b[39m "


class Recorder:
    """pexpect log hook: keeps every chunk the shell prints, with its time."""

    def __init__(self):
        self.events = []
        self.start = None

    def write(self, data):
        if self.start is not None:
            self.events.append([round(time.time() - self.start, 3), "o", data])

    def flush(self):
        pass


def main():
    work = Path(tempfile.mkdtemp(prefix="pgt-demo-", dir="/tmp"))
    # Copies, so rebuilding the project during the demo does not replace the running binary.
    bindir = work / "bin"
    bindir.mkdir()
    for name in ("pegada-term", "mock-energibridge"):
        shutil.copy(ROOT / "target" / "release" / name, bindir / name)
    real_home = Path.home()
    env = {
        "HOME": str(work),
        "ZDOTDIR": str(work),
        "PATH": f"{bindir}:{real_home}/.cargo/bin:/usr/bin:/bin:/usr/sbin:/sbin",
        "CARGO_HOME": str(real_home / ".cargo"),
        "RUSTUP_HOME": str(real_home / ".rustup"),
        "TERM": "xterm-256color",
        "LANG": "en_US.UTF-8",
        "PEGADA_TERM_RUNTIME_DIR": str(work / "rt"),
        "PEGADA_TERM_STATE_DIR": str(work / "state"),
        "PEGADA_TERM_ENERGIBRIDGE": os.environ.get("PEGADA_TERM_ENERGIBRIDGE", str(bindir / "mock-energibridge")),
        "PEGADA_TERM_INTERVAL": "500",
    }
    (work / ".zshrc").write_text(
        "unsetopt PROMPT_SP\nunset zle_bracketed_paste\n"
        "PS1='%F{71}❯%f '\n"
        'eval "$(pegada-term init zsh)"\n'
    )
    recorder = Recorder()
    child = pexpect.spawn(
        "zsh", ["-i"], env=env, encoding="utf-8", timeout=180, dimensions=(ROWS, COLS), cwd=str(ROOT)
    )
    try:
        child.expect_exact(PROMPT)
        time.sleep(8)  # sampler warm-up and idle baseline, not recorded
        subprocess.run(["touch", str(ROOT / "src" / "main.rs")], check=True)
        child.logfile_read = recorder
        recorder.start = time.time()
        recorder.write("\x1b[38;5;71m❯\x1b[39m ")

        def wait(seconds):
            """Sleeps while still reading, so every chunk is recorded when it arrives."""
            deadline = time.time() + seconds
            while time.time() < deadline:
                try:
                    child.read_nonblocking(4096, timeout=0.02)
                except pexpect.TIMEOUT:
                    pass

        def run(command, pause=1.5):
            wait(0.8)
            for ch in command:
                child.send(ch)
                wait(0.06)
            wait(0.4)
            child.send("\r")
            child.expect_exact(PROMPT)
            wait(pause)

        run("ls")
        run("sleep 5")
        run("cargo build --release --quiet")
        run("pegada-term stats")
        run("pegada-term status", pause=4)
    finally:
        child.logfile_read = None
        child.sendline("exit")
        child.close(force=True)
        subprocess.run([str(bindir / "pegada-term"), "stop"], env=env, capture_output=True)
        shutil.rmtree(work, ignore_errors=True)

    cast = ROOT / "docs" / "demo.cast"
    header = {"version": 2, "width": COLS, "height": ROWS, "env": {"TERM": "xterm-256color", "SHELL": "zsh"}}
    with cast.open("w") as f:
        f.write(json.dumps(header) + "\n")
        for event in recorder.events:
            f.write(json.dumps(event, ensure_ascii=False) + "\n")
    print(f"wrote {cast}")
    if shutil.which("agg"):
        gif = ROOT / "docs" / "demo.gif"
        subprocess.run(
            ["agg", "--idle-time-limit", "2", "--font-size", "16", "--theme", "monokai", str(cast), str(gif)],
            check=True,
        )
        print(f"wrote {gif}")
    else:
        print("agg not found: no GIF", file=sys.stderr)


if __name__ == "__main__":
    main()
