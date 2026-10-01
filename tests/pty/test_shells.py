"""End-to-end tests: real interactive zsh, bash and fish in a PTY, mock sensor.

Build first: cargo build --release --workspace
"""

import base64
import os
import re
import time

import pytest

from harness import MOCK, Sandbox, available, joules, strip_ansi

ALL = ["zsh", "zsh-nofd", "bash", "bash3", "fish"]
SHELLS = [pytest.param(s, marks=pytest.mark.skipif(not available(s), reason=f"{s} not installed")) for s in ALL]
BURN = f"{MOCK} --burn 2 4"
HINT = "sensor unavailable"


@pytest.fixture
def sandbox(request):
    kwargs = getattr(request, "param", {})
    sb = Sandbox(**kwargs)
    yield sb
    sb.close()


def ready(sandbox, name):
    sh = sandbox.shell(name)
    sandbox.wait_fresh()
    return sh


def raw_energy_line(sh, command):
    """The pegada-term line with its escape sequences (the shell prints others of its own)."""
    lines = [l for l in sh.run_raw(command).split("\n") if " total " in l]
    assert lines, "no energy line"
    return lines[-1]


@pytest.mark.parametrize("name", SHELLS)
def test_sleep_and_load_differ(sandbox, name):
    sh = ready(sandbox, name)
    time.sleep(3)  # let the idle baseline settle
    # The mock's power is 5 W + 10 W per core that `--burn` says it keeps busy
    # (MOCK_EB_LOAD_FILE), so other load on the machine does not count. Rounds
    # are still repeated in case a slow machine delays the sampler; the numbers
    # of the last round are reported.
    problems = []
    for _ in range(3):
        sleeps = [sh.energy_line("sleep 2") for _ in range(2)]
        burn = sh.energy_line(BURN)
        assert all(sleeps) and burn, (sleeps, burn)
        assert "2.0 s" in sleeps[0] or "2.1 s" in sleeps[0]
        problems = load_problems(sleeps, burn)
        if not problems:
            return
    pytest.fail(f"{problems}\nsleep: {sleeps}\nload: {burn}")


def load_problems(sleeps, burn):
    problems = []
    # 2 s cost at least 10 J, and at most what every core flat out would cost.
    ceiling = (5 + 10 * os.cpu_count()) * 2.6
    for line in sleeps + [burn]:
        if not 8 <= joules(line, "total") <= ceiling:
            problems.append(f"implausible total: {line}")
    # Take the quieter of the two sleeps.
    sleep_above = min(joules(l, "above idle") for l in sleeps)
    sleep_total = min(joules(l, "total") for l in sleeps)
    burn_above = joules(burn, "above idle")
    # "Above idle" counts whatever else the machine did meanwhile, so on a busy
    # desktop `sleep` is not exactly 0: allow the noise of two busy cores.
    if not sleep_above < 40:
        problems.append("sleep is not close to idle")
    if not (burn_above > sleep_above + 20 and burn_above > 2 * sleep_above):
        problems.append("4 busy cores are not clearly above sleep")
    if not joules(burn, "total") > sleep_total + 20:
        problems.append("total energy under load is not clearly above sleep")
    return problems


@pytest.mark.parametrize("name", SHELLS)
def test_short_commands_get_no_number(sandbox, name):
    sh = ready(sandbox, name)
    # Interval is 200 ms: under 400 ms there is no number at all.
    assert sh.energy_line("true") == "⚡ ·"
    # Between 2 and 4 intervals the values are marked approximate.
    line = sh.energy_line("sleep 0.55")
    assert line and "~" in line and "total" in line, line
    # From 4 intervals on they are not.
    line = sh.energy_line("sleep 1.2")
    assert line and "~" not in line, line


@pytest.mark.parametrize("name", SHELLS)
def test_exit_status_is_preserved(sandbox, name):
    sh = ready(sandbox, name)
    sh.run("sh -c 'exit 7'")
    assert "rc=7" in sh.run(f"echo rc={sh.status_var()}")
    sh.run("sh -c 'sleep 1; exit 3'")  # long enough to print the full line
    assert "rc=3" in sh.run(f"echo rc={sh.status_var()}")
    sh.run("true")
    assert "rc=0" in sh.run(f"echo rc={sh.status_var()}")


@pytest.mark.parametrize("name", SHELLS)
def test_empty_input_prints_nothing(sandbox, name):
    sh = ready(sandbox, name)
    sh.run("sleep 1")
    for _ in range(3):
        assert "⚡" not in sh.run("")


@pytest.mark.parametrize("name", SHELLS)
def test_styles_and_fallbacks(sandbox, name):
    sh = ready(sandbox, name)
    raw = raw_energy_line(sh, "sleep 1")
    assert "\x1b[38;5;" in raw and "\x1b[2m" in raw  # colours and dim by default

    sh.setenv("PEGADA_TERM_STYLE", "compact")
    line = sh.energy_line("sleep 1")
    assert line and "total" not in line and line.endswith(" J"), line

    sh.setenv("PEGADA_TERM_STYLE", "off")
    assert "⚡" not in sh.run("sleep 1")
    segment = sh.run("echo seg=$PEGADA_TERM_SEGMENT")
    assert "seg=⚡ " in segment and " J" in segment, segment

    sh.setenv("PEGADA_TERM_STYLE", "line")
    sh.setenv("NO_COLOR", "1")
    raw = raw_energy_line(sh, "sleep 1")
    assert "\x1b[38;5;" not in raw and "\x1b[2m" not in raw

    sh.setenv("PEGADA_TERM_ASCII", "1")
    line = sh.energy_line("sleep 1")
    assert line and line.startswith("* ") and "⚡" not in line and "·" not in line and "▁" not in line, line
    assert sh.energy_line("true") == "* ."

    sh.setenv("PEGADA_TERM_MIN_MS", "1500")
    assert sh.energy_line("sleep 1") is None
    assert sh.energy_line("true") is None
    assert sh.energy_line("sleep 1.7") is not None


def user_vars(raw):
    """OSC 1337 SetUserVar name -> decoded value, from raw terminal output."""
    found = re.findall(r"\x1b\]1337;SetUserVar=(\w+)=([A-Za-z0-9+/=]*)\x07", raw)
    return {name: base64.b64decode(value).decode() for name, value in found}


@pytest.mark.parametrize("sandbox", [{"extra_env": {"TERM_PROGRAM": "iTerm.app"}}], indirect=True)
@pytest.mark.parametrize("name", SHELLS)
def test_iterm_user_vars(sandbox, name):
    sh = ready(sandbox, name)
    raw = sh.run_raw("sleep 1")
    line = [l for l in strip_ansi(raw).splitlines() if " total " in l][-1]
    got = user_vars(raw)
    assert set(got) == {"pegadaTermLast", "pegadaTermAbove", "pegadaTermSession"}, raw
    # The status bar shows the same numbers as the line.
    assert f"⚡ {got['pegadaTermLast']} total" in line, (got, line)
    assert f"{got['pegadaTermAbove']} above idle" in line, (got, line)
    assert line.endswith(f"session {got['pegadaTermSession']}"), (got, line)
    assert "\x1bPtmux;" not in raw

    # Inside tmux each sequence is wrapped for passthrough, with ESC doubled.
    sh.setenv("TMUX", "/tmp/fake,1,0")
    raw = sh.run_raw("sleep 1")
    wrapped = re.findall(r"\x1bPtmux;\x1b(\x1b\]1337;SetUserVar=\w+=[A-Za-z0-9+/=]*\x07)\x1b\\", raw)
    assert len(wrapped) == 3, raw
    assert set(user_vars("".join(wrapped))) == set(got)


@pytest.mark.parametrize("name", SHELLS)
def test_no_user_vars_outside_iterm(sandbox, name):
    sh = ready(sandbox, name)
    assert "1337;SetUserVar" not in sh.run_raw("sleep 1")


@pytest.mark.parametrize("name", SHELLS)
def test_on_off(sandbox, name):
    sh = ready(sandbox, name)
    sh.run("pegada-term off")
    assert sh.energy_line("sleep 1") is None
    sh.run("pegada-term on")
    assert sh.energy_line("sleep 1") is not None


@pytest.mark.parametrize("name", SHELLS)
def test_history_keeps_no_arguments(sandbox, name):
    sh = ready(sandbox, name)
    sh.run("cargo build --features secret-feature 2>/dev/null; sleep 1")
    sh.run("sleep 0.5 0.5")
    sh.run("TOKEN=hunter2 sleep 1")
    sh.run("true")  # below resolution: not recorded
    rows = sandbox.history()
    assert [r[5] for r in rows] == ["cargo build", "sleep", "sleep"], rows
    text = (sandbox.state / "history.tsv").read_text()
    assert "secret" not in text and "hunter2" not in text and "0.5" not in text
    for epoch, dur, total, above, status, _ in rows:
        assert abs(int(epoch) - time.time()) < 60 and 900 < int(dur) < 1600
        assert int(total) >= int(above) >= 0 and status == "0"

    sh.setenv("PEGADA_TERM_HISTORY", "0")
    sh.run("sleep 1")
    assert len(sandbox.history()) == 3

    stats = sh.run("pegada-term stats all")
    assert "cargo build" in stats and "gCO₂" in stats and "phone charges" in stats, stats
    status = sh.run("pegada-term status")
    assert "running" in status and "in 4 commands" in status, status


@pytest.mark.parametrize("name", SHELLS)
def test_one_sampler_for_three_shells(sandbox, name):
    shells = [sandbox.shell(name) for _ in range(3)]
    sandbox.wait_fresh()
    for sh in shells:
        assert sh.energy_line("sleep 1")
    assert len(sandbox.daemons()) == 1
    assert len(sandbox.sensors()) == 1
    assert len(list((sandbox.runtime / "sessions").iterdir())) == 3

    shells[0].close()
    shells[1].close()
    time.sleep(4)
    assert len(sandbox.daemons()) == 1, "the sampler must stay while one shell is open"
    shells[2].close()
    # The watchdog checks every 3 s and leaves after 3 empty checks.
    assert sandbox.wait_for(lambda: not sandbox.daemons() and not sandbox.sensors(), timeout=25), sandbox.describe()
    assert not sandbox.pids("__watchdog")


@pytest.mark.parametrize("name", SHELLS)
def test_recovers_after_sensor_is_killed(sandbox, name):
    sh = ready(sandbox, name)
    (daemon,) = sandbox.daemons()
    (sensor,) = sandbox.sensors()
    energy_before = int(sandbox.read_state()[2])
    os.kill(sensor, 9)

    def restarted():
        sensors = sandbox.sensors()
        return len(sensors) == 1 and sensors[0] != sensor

    assert sandbox.wait_for(restarted, timeout=10), "the daemon did not restart the sensor"
    sandbox.wait_fresh()
    assert sandbox.daemons() == [daemon]
    assert int(sandbox.read_state()[2]) >= energy_before, "energy must stay monotonic across restarts"
    assert sh.energy_line("sleep 1")
    assert HINT not in sh.run("true")


@pytest.mark.parametrize("name", SHELLS)
def test_restart_by_hook_when_daemon_is_gone(sandbox, name):
    sh = ready(sandbox, name)
    (daemon,) = sandbox.daemons()
    os.kill(daemon, 9)
    assert sandbox.wait_for(lambda: not sandbox.sensors(), timeout=10)
    # Let the state file go stale. fish only has a whole-second clock for this,
    # so it needs up to a second more than the others.
    time.sleep(6)
    sh.run("true")  # the hook notices and starts a new sampler
    sh.run("true")
    assert sandbox.wait_for(lambda: len(sandbox.daemons()) == 1, timeout=10)
    sandbox.wait_fresh()
    assert sandbox.daemons() != [daemon]
    assert sh.energy_line("sleep 1")


@pytest.mark.parametrize("sandbox", [{"mode": "fail"}], indirect=True)
@pytest.mark.parametrize("name", SHELLS)
def test_no_spam_when_sensor_fails(sandbox, name):
    sh = sandbox.shell(name)
    out = sh.run("true")
    time.sleep(6)  # past the first backoff
    for _ in range(8):
        out += sh.run("true")
        out += sh.run("sleep 0.3")
    time.sleep(6)
    for _ in range(4):
        out += sh.run("true")
    assert out.count(HINT) == 1, out
    assert "⚡" not in out
    assert not sandbox.sensors()
    doctor = sh.run("pegada-term doctor")
    assert "keeps exiting" in doctor and "PermissionDenied" in doctor, doctor


@pytest.mark.parametrize(
    "sandbox,source",
    [({"mode": "watts"}, "smc-system"), ({"mode": "amd"}, "rapl-amd-package"), ({"mode": "rapl"}, "rapl-package+dram")],
    indirect=["sandbox"],
)
def test_sensor_modes(sandbox, source):
    sh = ready(sandbox, "zsh")
    assert sandbox.read_state()[6] == source
    line = sh.energy_line("sleep 1")
    assert line and 4 <= joules(line, "total") <= (5 + 10 * os.cpu_count()) * 1.6, line


@pytest.mark.parametrize("sandbox", [{"mode": "wrap"}], indirect=True)
def test_counter_wrap_is_not_a_spike(sandbox):
    # The mock's package counter starts 12 J below its 32-bit wrap.
    sh = ready(sandbox, "zsh")
    energies = []
    for _ in range(30):
        energies.append(int(sandbox.read_state()[2]))
        time.sleep(0.2)
    assert energies == sorted(energies)
    # 6 s at no more than every core flat out; a missed wrap would add 262 kJ.
    assert energies[-1] - energies[0] < (5 + 10 * os.cpu_count()) * 8 * 1000
    line = sh.energy_line("sleep 1")
    assert line and joules(line, "total") < (5 + 10 * os.cpu_count()) * 1.6


@pytest.mark.skipif(not (available("bash") or available("bash3")), reason="bash not installed")
@pytest.mark.parametrize("sandbox", [{"bashrc_pre": "PROMPT_COMMAND='pc=$?; echo \"pc=$pc\"; '"}], indirect=True)
@pytest.mark.parametrize("name", [s for s in ("bash", "bash3") if available(s)])
def test_bash_keeps_existing_prompt_command(sandbox, name):
    sh = ready(sandbox, name)
    out = sh.run("sh -c 'sleep 1; exit 3'")
    # The older PROMPT_COMMAND still runs, and still sees the command's status.
    assert "pc=3" in out and "⚡" in out, out
    assert "⚡" not in sh.run("")
