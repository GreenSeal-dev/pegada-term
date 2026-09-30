#!/bin/sh
# Installer round trip in a throwaway HOME: install from a local "release"
# server, check what it wrote, uninstall, and check the rc files are
# byte-identical to before.
#
#   cargo build --release && tests/install_test.sh [path-to-energibridge-tarball]
#
# Without a tarball the mock stands in for EnergiBridge.
set -eu
root=$(cd "$(dirname "$0")/.." && pwd)
bin=$root/target/release
work=$(mktemp -d /tmp/pgt-install.XXXXXX)
port=$((20000 + $$ % 20000))

case "$(uname -s)-$(uname -m)" in
  Darwin-arm64) target=aarch64-apple-darwin ;;
  Darwin-x86_64) target=x86_64-apple-darwin ;;
  Linux-x86_64) target=x86_64-unknown-linux-musl ;;
  Linux-aarch64) target=aarch64-unknown-linux-musl ;;
  *) echo "unsupported platform" >&2; exit 1 ;;
esac

# A fake release tree: <base>/releases/download/<tag>/<asset>
rel=$work/www/pegada-term/releases/download/v0.1.0
ebrel=$work/www/eb/releases/download/v0.0.7
mkdir -p "$rel" "$ebrel" "$work/home"
tar -czf "$rel/pegada-term-v0.1.0-$target.tar.gz" -C "$bin" pegada-term
if [ -n "${1:-}" ]; then
  cp "$1" "$ebrel/energibridge-v0.0.7-$target.tar.gz"
else
  mkdir "$work/ebsrc"
  cp "$bin/mock-energibridge" "$work/ebsrc/energibridge"
  tar -czf "$ebrel/energibridge-v0.0.7-$target.tar.gz" -C "$work/ebsrc" energibridge
fi
(cd "$work/www" && exec python3 -m http.server "$port" >/dev/null 2>&1) &
server=$!
cleanup() {
  kill "$server" 2>/dev/null || true
  wait "$server" 2>/dev/null || true
  PEGADA_TERM_RUNTIME_DIR="$work/rt" "$bin/pegada-term" stop >/dev/null 2>&1 || true
  rm -rf "$work"
}
trap cleanup EXIT
sleep 1

export HOME="$work/home" ZDOTDIR="$work/home"
export PEGADA_TERM_RUNTIME_DIR="$work/rt"
export PEGADA_TERM_BASE_URL="http://127.0.0.1:$port/pegada-term"
export PEGADA_TERM_EB_BASE_URL="http://127.0.0.1:$port/eb"
export PEGADA_TERM_NO_SETUP=1 PEGADA_TERM_YES=1
unset XDG_DATA_HOME XDG_STATE_HOME XDG_CONFIG_HOME PEGADA_TERM_ENERGIBRIDGE PEGADA_TERM_STATE_DIR
PATH=/usr/bin:/bin:/usr/sbin:/sbin
# fish is optional; zsh and bash come with the system.
fish_dir=$(dirname "$(PATH="$PATH:/opt/homebrew/bin:/usr/local/bin" command -v fish 2>/dev/null || echo /nonexistent/x)")
[ -d "$fish_dir" ] && PATH="$PATH:$fish_dir"
export PATH

# Existing rc files: one ending in a newline, one not.
printf 'export A=1\n' >"$HOME/.zshrc"
printf 'export B=2' >"$HOME/.bashrc"
cp "$HOME/.zshrc" "$work/zshrc.orig"
cp "$HOME/.bashrc" "$work/bashrc.orig"

fail() { echo "FAIL: $*" >&2; exit 1; }

# Piped, as with curl | sh.
sh <"$root/install.sh" | tee "$work/install.log"
[ -x "$HOME/.local/bin/pegada-term" ] || fail "binary not installed"
[ -x "$HOME/.local/share/pegada-term/energibridge" ] || fail "EnergiBridge not installed"
grep -q '>>> pegada-term >>>' "$HOME/.zshrc" || fail "no block in .zshrc"
grep -q 'init bash' "$HOME/.bashrc" || fail "no block in .bashrc"
grep -q 'is installed' "$work/install.log" || fail "no banner"
if [ -z "${1:-}" ]; then
  grep -q 'sensor OK: rapl-package+dram' "$work/install.log" || fail "smoke test did not pass"
fi

# A second run must not add a second block.
sh "$root/install.sh" >/dev/null
[ "$(grep -c '>>> pegada-term >>>' "$HOME/.zshrc")" = 1 ] || fail "block added twice"

# The hook loads in a real shell.
zsh -ic 'typeset -f __pegada_term_precmd >/dev/null' 2>/dev/null || fail "hook not loaded by zsh"

"$HOME/.local/bin/pegada-term" uninstall --yes
cmp "$HOME/.zshrc" "$work/zshrc.orig" || fail ".zshrc not restored"
cmp "$HOME/.bashrc" "$work/bashrc.orig" || fail ".bashrc not restored"
[ ! -e "$HOME/.local/bin/pegada-term" ] || fail "binary still there"
[ ! -e "$HOME/.local/share/pegada-term" ] || fail "data dir still there"
[ ! -e "$HOME/.config/fish/conf.d/pegada-term.fish" ] || fail "fish conf still there"
echo "install/uninstall round trip OK"
