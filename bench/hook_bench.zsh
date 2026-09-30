# Times the pegada-term zsh hook: preexec + precmd, output to /dev/null.
# Run through bench/hook_bench.sh, which provides the sandbox.
zmodload zsh/datetime
eval "$(pegada-term init zsh)"
N=${1:-1000}
while [[ ! -s $__pegada_term_state ]]; do sleep 0.2; done
sleep 3  # some history for the sparkline
__pegada_term_osc=0  # measured separately below

bench() {  # $1: label, $2: 1 = pretend the command ran for 5 s
  local -i i long=$2
  local -F t0=$EPOCHREALTIME
  for (( i = 0; i < N; i++ )); do
    __pegada_term_preexec "cargo build --release"
    if (( long )); then (( __pegada_term_t0 -= 5000, __pegada_term_e0 = __pegada_term_e0 > 60000 ? __pegada_term_e0 - 60000 : 0 )); fi
    __pegada_term_precmd
  done > /dev/null
  printf '%-52s %6.0f us per prompt\n' "zsh $ZSH_VERSION, $1" $(( (EPOCHREALTIME - t0) * 1e6 / N ))
}

# The same, but with a real process between prompts, as in real use: the CPU
# has left the shell's code and files by the time the next hook runs.
bench_spaced() {
  local -i i long=$2 n=$(( N / 5 ))
  local -F t0 total=0
  for (( i = 0; i < n; i++ )); do
    t0=$EPOCHREALTIME
    __pegada_term_preexec "cargo build --release"
    (( total += EPOCHREALTIME - t0 ))
    sleep 0.05
    if (( long )); then (( __pegada_term_t0 -= 5000, __pegada_term_e0 = __pegada_term_e0 > 60000 ? __pegada_term_e0 - 60000 : 0 )); fi
    t0=$EPOCHREALTIME
    __pegada_term_precmd
    (( total += EPOCHREALTIME - t0 ))
  done > /dev/null
  printf '%-52s %6.0f us per prompt\n' "zsh $ZSH_VERSION, $1" $(( total * 1e6 / n ))
}
bench 'short command (no number)' 0
bench 'full line with sparkline + history' 1
PEGADA_TERM_STYLE=compact bench 'compact style + history' 1
PEGADA_TERM_HISTORY=0 bench 'full line, history off' 1
__pegada_term_osc=1 bench 'full line + iTerm2 user vars (OSC 1337)' 1
# What two cold wake-ups cost with no hook at all, for comparison.
__pegada_term_noop() { local -i x=$1; (( x++ )) }
(){
  local -i i n=$(( N / 5 ))
  local -F t0 total=0
  for (( i = 0; i < n; i++ )); do
    t0=$EPOCHREALTIME; __pegada_term_noop $i; (( total += EPOCHREALTIME - t0 ))
    sleep 0.05
    t0=$EPOCHREALTIME; __pegada_term_noop $i; (( total += EPOCHREALTIME - t0 ))
  done
  printf '%-52s %6.0f us per prompt\n' "zsh $ZSH_VERSION, spaced: two empty functions (baseline)" $(( total * 1e6 / n ))
}
bench_spaced 'spaced: short command (no number)' 0
bench_spaced 'spaced: full line with sparkline + history' 1
