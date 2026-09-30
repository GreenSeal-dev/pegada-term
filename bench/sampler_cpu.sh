#!/bin/sh
# Sampler overhead: CPU time used by the daemon, EnergiBridge and the watchdog
# over a fixed wall-clock window, at each sampling interval.
#
#   bench/sampler_cpu.sh [seconds=120] [intervals="200 500"]
#
# Uses the real EnergiBridge (set PEGADA_TERM_ENERGIBRIDGE to choose one).
set -eu
root=$(cd "$(dirname "$0")/.." && pwd)
bin=$root/target/release/pegada-term
secs=${1:-120}
intervals=${2:-"200 500"}

cpu_seconds() {  # total CPU seconds of the given pids
  ps -o time= -p "$(echo "$@" | tr ' ' ',')" | awk -F'[:.]' '
    { n = NF; s = $(n-1) + $n / 100; if (n > 2) s += $(n-2) * 60; if (n > 3) s += $(n-3) * 3600; t += s }
    END { printf "%.2f\n", t }'
}

for interval in $intervals; do
  tmp=$(mktemp -d /tmp/pgt-cpu.XXXXXX)
  export PEGADA_TERM_RUNTIME_DIR="$tmp/rt" PEGADA_TERM_STATE_DIR="$tmp/state" PEGADA_TERM_INTERVAL="$interval"
  mkdir -p "$tmp/rt/sessions"
  : > "$tmp/rt/sessions/$$"          # this script is the "shell" that keeps the sampler alive
  "$bin" daemon --start
  sleep 5
  daemon=$(cat "$tmp/rt/daemon.lock")
  sensor=$(cat "$tmp/rt/sensor.pid")
  watchdog=$(cat "$tmp/rt/watchdog.pid")
  before=$(cpu_seconds "$daemon" "$sensor" "$watchdog")
  each_before="$(cpu_seconds "$daemon") $(cpu_seconds "$sensor") $(cpu_seconds "$watchdog")"
  sleep "$secs"
  after=$(cpu_seconds "$daemon" "$sensor" "$watchdog")
  each_after="$(cpu_seconds "$daemon") $(cpu_seconds "$sensor") $(cpu_seconds "$watchdog")"
  source=$(head -n 1 "$tmp/rt/state" | cut -d' ' -f7)
  echo "$before $after $secs $each_before $each_after" | awk -v i="$interval" -v src="$source" '{
    printf "interval %4d ms (%s): %.3f%% of one core  [daemon %.3f%%, energibridge %.3f%%, watchdog %.3f%%]  (%.2f s CPU in %d s)\n",
      i, src, ($2 - $1) * 100 / $3, ($7 - $4) * 100 / $3, ($8 - $5) * 100 / $3, ($9 - $6) * 100 / $3, $2 - $1, $3 }'
  "$bin" stop >/dev/null
  rm -rf "$tmp"
done
