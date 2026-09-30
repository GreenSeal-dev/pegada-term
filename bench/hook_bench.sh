#!/bin/sh
# Hook overhead: microseconds added per prompt (preexec + precmd), measured
# over N iterations against a live sampler fed by mock-energibridge.
#
#   cargo build --release --workspace && bench/hook_bench.sh [N]
set -eu
root=$(cd "$(dirname "$0")/.." && pwd)
bin=$root/target/release
n=${1:-1000}
tmp=$(mktemp -d /tmp/pgt-bench.XXXXXX)
export PATH="$bin:$PATH"
export PEGADA_TERM_RUNTIME_DIR="$tmp/rt" PEGADA_TERM_STATE_DIR="$tmp/state"
export PEGADA_TERM_ENERGIBRIDGE="$bin/mock-energibridge" PEGADA_TERM_INTERVAL=500
export LANG="${LANG:-en_US.UTF-8}" COLUMNS=120
unset NO_COLOR PEGADA_TERM_STYLE PEGADA_TERM_MIN_MS PEGADA_TERM_ASCII PEGADA_TERM_HISTORY
trap 'pegada-term stop >/dev/null 2>&1; rm -rf "$tmp"' EXIT

command -v zsh >/dev/null && zsh -f "$root/bench/hook_bench.zsh" "$n"
for b in /opt/homebrew/bin/bash /usr/local/bin/bash /bin/bash; do
  [ -x "$b" ] && "$b" --norc --noprofile "$root/bench/hook_bench.bash" "$n"
done
