# pegada-term

[![AI coding CO2e: 0.0785–3.02 kg](https://img.shields.io/badge/AI%20coding%20CO2e-0.0785%E2%80%933.02%20kg-4c8c4a)](https://github.com/GreenSeal-dev/pegada-code/blob/main/METHODOLOGY.md)

**See the energy used by every command you run in the terminal.**

![pegada-term in zsh: an energy line after each command](docs/demo.gif)

After each command, one line before your prompt:

```
⚡ 142 J total · 38 J above idle · 12.4 s · 11.5 W  ▂▃▅▇█▇▅▃▂  · session 3.2 kJ
```

*Pegada* is Portuguese for *footprint*. pegada-term is part of the pegada family, next to
[pegada-code](https://github.com/GreenSeal-dev/pegada-code). It reads your machine's power sensor
through [EnergiBridge](https://github.com/tdurieux/EnergiBridge) and works in zsh, bash and fish, on
macOS and Linux.

- **Cheap.** No process is started per command. The hooks are shell builtins reading a small file;
  one background sampler is shared by all your shells and exits with the last one.
  [Measured overhead](#overhead) is below.
- **Honest.** The numbers are whole-machine energy while the command ran, not per-process
  attribution, and commands too short to measure get no number. See
  [What is measured](#what-is-measured).

## Install

```sh
curl -fsSL https://raw.githubusercontent.com/GreenSeal-dev/pegada-term/main/install.sh | sh
```

The installer:

1. puts the `pegada-term` binary in `~/.local/bin`;
2. uses the `energibridge` on your `PATH` if there is one, otherwise downloads the matching
   EnergiBridge release to `~/.local/share/pegada-term/`;
3. on Linux, explains and asks before running `sudo pegada-term setup` (see
   [Linux setup](#linux-setup-and-its-security-trade-off));
4. adds a marked block to `~/.zshrc` and `~/.bashrc`, and a `conf.d` file for fish;
5. tests the sensor and prints, for example, `sensor OK: smc-system, 7.9 W idle`.

Then open a new terminal.

With cargo instead: `cargo install pegada-term`, install EnergiBridge yourself, and add one line to
your shell's rc file:

```sh
eval "$(pegada-term init zsh)"      # ~/.zshrc
eval "$(pegada-term init bash)"     # ~/.bashrc
pegada-term init fish | source      # ~/.config/fish/config.fish
```

Put it after other prompt tools (starship, oh-my-zsh, bash-preexec), so they do not replace its hook.

To remove everything: `pegada-term uninstall`. It asks before using sudo, leaves your rc files
byte-identical to before the install, and keeps your history unless you pass `--purge`.

| Platform | Sensor | Status |
|---|---|---|
| macOS, Apple Silicon | SMC `SYSTEM_POWER` | Works. Developed and tested on this. |
| macOS, Intel | SMC `SYSTEM_POWER` / `CPU_POWER` | Should work; not tested on hardware. |
| Linux, Intel or AMD x86-64 | RAPL through MSRs | Needs `sudo pegada-term setup`. Tested against a mock sensor and in CI, not yet on hardware. |
| Linux on ARM | none | EnergiBridge has no sensor for it; pegada-term installs but shows no numbers. |
| VMs, containers, WSL | none | No MSR access. `pegada-term doctor` says so. |
| Windows | — | Not supported. |

## What is measured

Read this before quoting a number.

- **Total** is the energy the **whole machine** used between the moment the command started and the
  moment it ended. On Apple Silicon that is system power from the SMC, display included. On Linux it
  is the CPU package plus DRAM from RAPL, so no display, disk, GPU or power-supply losses. Total
  includes the power the machine would have drawn doing nothing.
- **Above idle** is an *estimate* of what the command added:
  `max(0, total − idle power × duration)`. Idle power is the 10th percentile of power over the last
  five minutes, using only moments when none of your shells was running a command.
- **Neither is per-process attribution.** If a browser tab spins up while `make` runs, it is in
  both numbers. If two shells run commands at once, each reports the machine's energy, so adding
  them up counts it twice (`pegada-term stats` does add them up).
- **Short commands get no number.** The sensor is sampled every 500 ms, and the Apple Silicon SMC
  only refreshes its value about once a second. A command shorter than two of those periods is
  below the measurement's resolution and shows a dim `⚡ ·`. Between two and four periods, values
  are prefixed with `~`. `pegada-term status` shows the real update period of your sensor.
- **The Apple Silicon sensor lags.** On an M-series MacBook Pro, a step in CPU load showed up in
  `SYSTEM_POWER` 2–3 s later, and dropped 2–3 s after the load stopped. So a 5 s burst is partly
  charged to whatever you run next. Numbers for commands over about 10 s are little affected; for
  shorter ones, treat them as rough. RAPL on Linux does not have this lag.
- **Interpolation.** Energy at the start and end of a command is
  `E(sample) + P(sample) × (t − t_sample)`, capped at two sample intervals.
- **fish** has no millisecond clock, so there the energy is `$CMD_DURATION ×` the average power
  between the sampler's state at the start and at the end of the command.
- **Gaps are not integrated.** If the machine sleeps mid-command, the time asleep counts in the
  duration but adds no energy.
- **gCO₂** in `stats` is `energy × PEGADA_TERM_CARBON_INTENSITY`, 250 gCO₂/kWh unless you set it.
  That default is a round figure, not your grid.

## Commands

| | |
|---|---|
| `pegada-term` / `pegada-term status` | Sampler state, sensor, live power, idle baseline, today's and this session's totals |
| `pegada-term stats [today\|week\|all]` | Top commands by energy (total and above idle), totals in Wh, gCO₂ and phone charges |
| `pegada-term watch` | Full-screen live power meter with the idle baseline |
| `pegada-term doctor` | Checks platform, EnergiBridge, permissions, sampler CPU time and the log; prints the exact fix |
| `pegada-term on` / `off` | Show or hide the line in this shell |
| `pegada-term start` / `stop` / `restart` | Control the sampler (it normally starts and stops by itself) |
| `pegada-term setup` | Linux: the privileged MSR setup. macOS: a check only |
| `pegada-term uninstall [--purge]` | Remove everything |

## Configuration

Environment variables, read on every prompt unless noted:

| Variable | Default | |
|---|---|---|
| `PEGADA_TERM_STYLE` | `line` | `line`: the full line. `compact`: `⚡ 142 J`. `off`: nothing printed |
| `PEGADA_TERM_MIN_MS` | `0` | Print nothing for commands shorter than this |
| `PEGADA_TERM_INTERVAL` | `500` | Sampling interval in ms (100–10000). Read when the sampler starts; `pegada-term restart` to apply. EnergiBridge warns below 200 |
| `PEGADA_TERM_HISTORY` | `1` | `0` stops recording to `history.tsv` |
| `PEGADA_TERM_CARBON_INTENSITY` | `250` | gCO₂/kWh, used by `stats` only |
| `PEGADA_TERM_ASCII` | unset | `1` forces ASCII. Also automatic when the locale is not UTF-8 |
| `NO_COLOR` | unset | Any value turns colours off |

The "above idle" value is green below 100 J, amber below 1 kJ and red from there. The sparkline
goes from the idle baseline to the command's peak.

### In your prompt

`$PEGADA_TERM_SEGMENT` always holds the last result (`⚡ 142 J`), whatever the style, so you can set
`PEGADA_TERM_STYLE=off` and show it in your prompt instead.

```zsh
# powerlevel10k: add `pegada` to POWERLEVEL9K_RIGHT_PROMPT_ELEMENTS
function prompt_pegada() {
  [[ -n $PEGADA_TERM_SEGMENT ]] && p10k segment -f 71 -t "$PEGADA_TERM_SEGMENT"
}
```

```toml
# starship.toml
[custom.pegada]
command = 'echo $PEGADA_TERM_SEGMENT'
when = 'test -n "$PEGADA_TERM_SEGMENT"'
```

### iTerm2 and WezTerm status bar

In iTerm2 and WezTerm the hook also sets three user variables through `OSC 1337 SetUserVar`:
`pegadaTermLast`, `pegadaTermAbove` and `pegadaTermSession`.

In iTerm2: **Settings → Profiles → Session → Status bar enabled → Configure Status Bar**, drag in an
**Interpolated String** component, and set its value to:

```
⚡ \(user.pegadaTermLast) · \(user.pegadaTermAbove) above idle · session \(user.pegadaTermSession)
```

Inside tmux the sequences are wrapped for passthrough; tmux 3.3 or later needs
`set -g allow-passthrough on`.

## How it works

```
 zsh / bash / fish hooks                 pegada-term daemon (one per user)
 ───────────────────────                 ──────────────────────────────────
 preexec: read state, note E(start) ◄──  state file, rewritten every sample
 precmd:  read state, E(end) − E(start)      ▲
          print the line                     │ CSV on a pipe
 mark busy / idle in sessions/<pid> ──►  energibridge -i 500 -- pegada-term __watchdog
```

- `eval "$(pegada-term init zsh)"` costs one process at shell startup. The hook code is compiled
  into the binary, so the two cannot drift apart.
- The **sampler** starts with the first shell, detaches (`setsid`), and holds a `flock`.
- EnergiBridge measures a command until it exits. The command here is the **watchdog**, which
  checks every 3 s whether any registered shell is still alive and exits after about 9 s without
  one; EnergiBridge and the daemon exit with it.
- EnergiBridge v0.0.7 panics if one sample takes longer than the interval, and on Linux if an MSR
  cannot be read. The daemon restarts it, keeps the energy counter monotonic across restarts, and
  gives up after three starts that fail within 5 s. The hooks then show one hint per session
  (`sensor unavailable - run pegada-term doctor`) and retry with a growing backoff.
- RAPL counters are 32-bit and wrap. A negative delta is treated as a wrap and replaced by the
  last power × elapsed time; so is any reading above 10 kW.

Files:

| | |
|---|---|
| `$XDG_RUNTIME_DIR/pegada-term/` (Linux), `$TMPDIR/pegada-term-$UID/` (macOS) | `state`, `sessions/<pid>`, lock and pid files |
| `$XDG_STATE_HOME/pegada-term/` (default `~/.local/state/pegada-term/`) | `history.tsv`, `daemon.log` |

State file, line 1: `seq t_ms energy_mJ power_mW idle_mW interval_ms source res_ms seq`. Lines 2–4:
power history in mW, newest first, at 1×, 10× and 100× the sample interval, 64 entries each.

Two things differ from the obvious design, both for overhead:

- The state file is **rewritten in place with one `pwrite`**, not replaced through a temp file and
  `rename`. On macOS, create + rename cost about 1.6 ms of CPU per sample against 0.1 ms, and an
  unchanged inode lets zsh keep the file open. `seq` is repeated at the end of line 1 so a reader
  can reject a line caught mid-write.
- A shell marks itself busy by **appending one byte** to its session file when a command starts and
  one when it ends: odd length means busy.

### History and privacy

`history.tsv` holds one line per measured command:
`epoch  duration_ms  total_mJ  above_idle_mJ  exit_status  command_head`.
`command_head` is the first word only, plus the subcommand for a fixed list of tools such as `git`,
`cargo`, `docker` or `kubectl` (`cargo build`, `git push`). Arguments, paths and variable
assignments are never recorded. Commands below the sensor's resolution are not recorded.
Nothing leaves your machine.

## Overhead

Measured on a MacBook Pro (Apple Silicon, macOS 26, zsh 5.9, bash 5.3) with `bench/hook_bench.sh`,
`bench/sampler_cpu.sh` and `bench/ab_idle.sh`. Reproduce them on your machine; the machine was in
normal desktop use, so expect some spread.

**Per prompt** (`preexec` + `precmd`, 1000 iterations, output to `/dev/null`):

| | zsh 5.9 | bash 5.3 |
|---|---|---|
| Short command (`⚡ ·`) | 232 µs | 368 µs |
| `compact` style, with history | 271 µs | 473 µs |
| Full line with sparkline and history | 377 µs | 754 µs |
| Full line plus iTerm2 user variables | 504 µs | 975 µs |

That is under the 1 ms target, but it is the warm case: a tight loop keeps the CPU fast and the
caches full. With a 50 ms pause and a process between prompts, as in real use, the same code took
**2.1 ms (short) to 3.4 ms (full line) in zsh** and 3.4 to 6.2 ms in bash, on a machine where two
empty shell functions took 0.3–0.4 ms under the same conditions. Everything the shell does right
after waking is that much slower, so the realistic cost is a few milliseconds per prompt, not
under one.

**bash 3.2** (the `/bin/bash` on macOS) has no sub-second clock, so each timestamp forks `perl`:
about 15 ms per prompt. Use bash 5 or zsh if that matters.

**Sampler** (daemon + EnergiBridge + watchdog, CPU time over 5 minutes):

| Interval | Total | daemon | EnergiBridge | watchdog |
|---|---|---|---|---|
| 200 ms | 1.81 % of one core | 0.14 % | 1.66 % | 0.02 % |
| 500 ms (default) | 0.83 % of one core | 0.07 % | 0.74 % | 0.02 % |

EnergiBridge is nine tenths of it: on every sample it also collects per-core usage, frequency,
temperature and memory, and on macOS opens a new SMC connection. See [v2](#v2).

**Idle power, sampler on vs off:** no difference detected. Off 9379 mW (sd 794), on 9486 mW
(sd 1108), difference **+106 mW, 95 % interval −738 to +951 mW** (Welch). 10 alternating one-minute
runs per arm at the default 500 ms interval, MacBook on AC, fully charged. The run-to-run noise of
this machine (about 1 W) is far larger than the effect, so this only bounds the cost below roughly
1 W; the CPU figures above suggest the true cost is a small fraction of that. The meter was the
weaker one: a second EnergiBridge at 5 s reading the same SMC sensor in both arms
(`bench/ab_idle.sh --no-sudo 10 60 500`), and the machine was in light use during the run. For an
independent meter, run `sudo bench/ab_idle.sh` (powermetrics) on a quiet machine.

## v2

v1 talks to EnergiBridge as a process: spawn it, parse its CSV. That sits behind a `Sensor` trait
(`src/sensor/`). Once EnergiBridge exposes its sensor code as a library crate, an `EnergiBridgeLib`
implementation can replace `EnergiBridgeProcess`, and would:

- **read only the energy counters**, skipping the per-core statistics and the per-sample SMC
  connection that make up most of the sampler's CPU time today (1.66 of 1.81 % at 200 ms);
- **sample adaptively**: fast while a command runs, slow or paused at the prompt. The hooks would
  signal the daemon with the builtin `kill`, so still no fork. At the prompt the sampler would then
  cost close to nothing;
- on Linux, **read RAPL at the moment each command starts and ends**, which removes the
  interpolation and the resolution limit for short commands;
- remove one process and the watchdog.

## Linux setup and its security trade-off

EnergiBridge reads RAPL from `/dev/cpu/N/msr`, not from powercap. That needs the `msr` kernel
module, read permission on the devices, and `CAP_SYS_RAWIO`. `sudo pegada-term setup`:

- creates the group `msr`;
- installs EnergiBridge as `/usr/local/lib/pegada-term/energibridge`, owned by root, setgid `msr`
  (mode 2755), with `setcap cap_sys_rawio=ep`, so no re-login is needed;
- adds the udev rule `SUBSYSTEM=="msr", KERNEL=="msr[0-9]*", GROUP="msr", MODE="0640"`;
- adds `/etc/modules-load.d/pegada-term-msr.conf`, runs `modprobe msr` and applies the permissions
  to the existing devices.

**Trade-off:** RAPL is a power side channel (PLATYPUS, CVE-2020-8694), which is why Linux makes it
root-only. After this setup any local user can read RAPL *through EnergiBridge*. Programs that
EnergiBridge starts inherit the group but not the capability, so they cannot open the MSR devices
themselves. Do not run the setup on a shared or multi-tenant machine.

`pegada-term uninstall` removes these files after asking. It leaves the `msr` group in place.

## Troubleshooting

Run `pegada-term doctor`. It reports the platform, virtualization, the EnergiBridge binary and
version, MSR module and permissions on Linux, whether the sampler runs and how much CPU time it
has used, the last log lines, and the command that fixes what it found.

- **No line at all:** the sensor is unavailable, and you got the one-line hint earlier in the
  session. `doctor` says why.
- **Only `⚡ ·`:** the commands are shorter than the sensor's resolution.
- **The line appears twice or not at all in bash:** something else sets `PROMPT_COMMAND` or a
  `DEBUG` trap after pegada-term. Move the `eval` line to the end of `~/.bashrc`.

## Development

```sh
cargo build --release --workspace     # pegada-term and mock-energibridge
cargo test --workspace                # CSV, integration, wraps, idle percentile, state format
python -m pytest tests/pty            # real zsh, bash and fish in a PTY (pip install pytest pexpect)
tests/install_test.sh                 # installer and uninstaller round trip in a throwaway HOME
bench/hook_bench.sh                   # microseconds per prompt
bench/sampler_cpu.sh                  # sampler CPU use
sudo bench/ab_idle.sh                 # idle power with the sampler on vs off
```

`tools/mock-energibridge` stands in for EnergiBridge in the tests. It has the same CLI and CSV
format, with power derived from the machine's real CPU load (`5 W + 10 W × busy cores`), and
`MOCK_EB_MODE` selects `rapl`, `amd`, `watts`, `wrap` (a counter about to wrap) or `fail`. With
`MOCK_EB_LOAD_FILE` set, power follows only the load that `--burn` announces, which keeps the PTY
tests independent of other load on a shared CI machine.

EnergiBridge used to abort a measurement when one sample took longer than the interval; that is
fixed upstream in [tdurieux/EnergiBridge#23](https://github.com/tdurieux/EnergiBridge/pull/23).
The installer pins EnergiBridge v0.0.7, which predates the fix, so the sampler's restart-on-crash
still matters until a newer EnergiBridge release is out.

Not done yet: a Homebrew tap, a PowerShell hook for native Windows, and the v2 sensor.

## License

MIT. See [LICENSE](LICENSE).
