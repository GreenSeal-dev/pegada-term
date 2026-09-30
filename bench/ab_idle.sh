#!/bin/sh
# Does the sampler change the machine's idle power? A/B test on real hardware.
#
#   sudo bench/ab_idle.sh [runs=10] [seconds=60] [interval_ms=500]
#   bench/ab_idle.sh --no-sudo [runs] [seconds] [interval_ms]
#
# Alternates runs with the sampler OFF and ON (order flipped every pair, to
# cancel drift). In each run an independent meter records average power:
#   default    `powermetrics` (macOS, needs root): SMC-independent CPU/GPU/ANE
#              "Combined Power", the part of the machine the sampler can load.
#   --no-sudo  a second EnergiBridge at a 5 s interval: whole-system power from
#              the same sensor the sampler reads. Weaker: not independent.
# The meter runs in both arms, so its own cost cancels out.
#
# For a clean result: on AC power, display on at fixed brightness, no other
# work, and leave the machine alone for the ~25 minutes it takes.
# Prints one line per run and the ON - OFF difference with a 95% interval.
set -eu

meter=powermetrics
if [ "${1:-}" = --no-sudo ]; then
  meter=energibridge
  shift
fi
runs=${1:-10}
secs=${2:-60}
interval=${3:-500}
root=$(cd "$(dirname "$0")/.." && pwd)
bin=$root/target/release/pegada-term
[ -x "$bin" ] || { echo "build first: cargo build --release" >&2; exit 1; }

tmp=$(mktemp -d /tmp/pgt-ab.XXXXXX)
export PEGADA_TERM_RUNTIME_DIR="$tmp/rt" PEGADA_TERM_STATE_DIR="$tmp/state" PEGADA_TERM_INTERVAL="$interval"
eb=${PEGADA_TERM_ENERGIBRIDGE:-}
if [ -z "$eb" ]; then
  for c in /usr/local/lib/pegada-term/energibridge "$HOME/.local/share/pegada-term/energibridge" "$(command -v energibridge || true)"; do
    if [ -n "$c" ] && [ -x "$c" ]; then eb=$c; break; fi
  done
fi
[ -n "$eb" ] || { echo "EnergiBridge not found; set PEGADA_TERM_ENERGIBRIDGE" >&2; exit 1; }
export PEGADA_TERM_ENERGIBRIDGE="$eb"
if [ "$meter" = powermetrics ]; then
  [ "$(uname -s)" = Darwin ] || { echo "powermetrics is macOS only; use --no-sudo" >&2; exit 1; }
  [ "$(id -u)" = 0 ] || { echo "powermetrics needs root: sudo $0 $*  (or --no-sudo)" >&2; exit 1; }
fi
trap '"$bin" stop >/dev/null 2>&1; rm -rf "$tmp"' EXIT
mkdir -p "$tmp/rt/sessions"

# Average power in mW over $secs seconds.
measure() {
  if [ "$meter" = powermetrics ]; then
    powermetrics --samplers cpu_power -i 1000 -n "$secs" 2>/dev/null |
      awk '/^Combined Power/ { s += $(NF-1); n++ } END { if (n) printf "%.1f\n", s / n; else print "nan" }'
  else
    "$eb" -i 5000 -- sleep "$secs" 2>/dev/null | awk -F, '
      NR == 1 { for (i = 1; i <= NF; i++) if ($i == "SYSTEM_POWER (Watts)" || $i == "CPU_POWER (Watts)") col = i; next }
      col && NR > 2 { s += $col; n++ }
      END { if (n) printf "%.1f\n", s * 1000 / n; else print "nan" }'
  fi
}

run() {  # $1 = on | off
  if [ "$1" = on ]; then
    : >"$tmp/rt/sessions/$$"   # this script is the "shell" keeping the sampler alive
    "$bin" daemon --start
  else
    "$bin" stop >/dev/null 2>&1 || true
    rm -f "$tmp/rt/sessions/$$"
  fi
  sleep 10   # settle
  mw=$(measure)
  echo "$1 $mw" | tee -a "$tmp/results"
}

echo "meter: $meter, $runs runs per arm, $secs s each, sampler interval $interval ms" >&2
i=0
while [ "$i" -lt "$runs" ]; do
  if [ $((i % 2)) = 0 ]; then run off; run on; else run on; run off; fi
  i=$((i + 1))
done
"$bin" stop >/dev/null 2>&1 || true

awk '
  $2 == "nan" { next }
  { n[$1]++; s[$1] += $2; q[$1] += $2 * $2 }
  END {
    if (n["on"] < 2 || n["off"] < 2) { print "not enough valid runs"; exit 1 }
    mon = s["on"] / n["on"]; moff = s["off"] / n["off"]
    von = (q["on"] - n["on"] * mon * mon) / (n["on"] - 1)
    voff = (q["off"] - n["off"] * moff * moff) / (n["off"] - 1)
    se = sqrt(von / n["on"] + voff / n["off"])
    printf "sampler off: %.1f mW (sd %.1f, n=%d)\n", moff, sqrt(voff), n["off"]
    printf "sampler on:  %.1f mW (sd %.1f, n=%d)\n", mon, sqrt(von), n["on"]
    printf "difference:  %+.1f mW (95%% interval %+.1f to %+.1f, Welch, normal approximation)\n", mon - moff, mon - moff - 1.96 * se, mon - moff + 1.96 * se
  }' "$tmp/results"
