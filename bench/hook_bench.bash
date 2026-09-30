# Times the pegada-term bash hook: preexec + precmd, output to /dev/null.
# Run through bench/hook_bench.sh, which provides the sandbox.
eval "$(pegada-term init bash)"
N=${1:-1000}
: > "$__pegada_term_session"  # non-interactive: register by hand
while [[ ! -s $__pegada_term_state ]]; do sleep 0.2; done
sleep 3  # some history for the sparkline
__pegada_term_osc=0  # measured separately below

now_us() {
  if [[ -n ${EPOCHREALTIME-} ]]; then
    local t=$EPOCHREALTIME
    us=${t%[.,]*}${t#*[.,]}
  else
    us=$(perl -MTime::HiRes=time -e 'printf "%d", time * 1e6')
  fi
}
bench() {  # $1: label, $2: 1 = pretend the command ran for 5 s
  local i long=$2 start
  now_us; start=$us
  for ((i = 0; i < N; i++)); do
    __pegada_term_preexec "cargo build --release"
    if ((long)); then ((__pegada_term_t0 -= 5000, __pegada_term_e0 = __pegada_term_e0 > 60000 ? __pegada_term_e0 - 60000 : 0)); fi
    __pegada_term_precmd
  done > /dev/null
  now_us
  printf '%-52s %6d us per prompt\n' "bash $BASH_VERSION, $1" $(((us - start) / N))
}

# The same, but with a real process between prompts, as in real use: the CPU
# has left the shell's code and files by the time the next hook runs.
bench_spaced() {
  local i long=$2 start total=0 n=$((N / 5))
  for ((i = 0; i < n; i++)); do
    now_us; start=$us
    __pegada_term_preexec "cargo build --release"
    now_us; ((total += us - start))
    sleep 0.05
    if ((long)); then ((__pegada_term_t0 -= 5000, __pegada_term_e0 = __pegada_term_e0 > 60000 ? __pegada_term_e0 - 60000 : 0)); fi
    now_us; start=$us
    __pegada_term_precmd
    now_us; ((total += us - start))
  done > /dev/null
  printf '%-52s %6d us per prompt\n' "bash $BASH_VERSION, $1" $((total / n))
}
bench 'short command (no number)' 0
bench 'full line with sparkline + history' 1
PEGADA_TERM_STYLE=compact bench 'compact style + history' 1
PEGADA_TERM_HISTORY=0 bench 'full line, history off' 1
__pegada_term_osc=1 bench 'full line + iTerm2 user vars (OSC 1337)' 1
# What two cold wake-ups cost with no hook at all, for comparison.
__pegada_term_noop() { local x=$1; ((x++)); }
baseline() {
  local i start total=0 n=$((N / 5))
  for ((i = 0; i < n; i++)); do
    now_us; start=$us; __pegada_term_noop "$i"; now_us; ((total += us - start))
    sleep 0.05
    now_us; start=$us; __pegada_term_noop "$i"; now_us; ((total += us - start))
  done
  printf '%-52s %6d us per prompt\n' "bash $BASH_VERSION, spaced: two empty functions (baseline)" $((total / n))
}
# bash 3.2 pays two perl forks per prompt for the clock; nothing more to learn there.
if [[ -n ${EPOCHREALTIME-} ]]; then
  baseline
  bench_spaced 'spaced: short command (no number)' 0
  bench_spaced 'spaced: full line with sparkline + history' 1
fi
