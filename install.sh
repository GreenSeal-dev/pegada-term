#!/bin/sh
# pegada-term installer.
#
#   curl -fsSL https://raw.githubusercontent.com/GreenSeal-dev/pegada-term/main/install.sh | sh
#
# Installs the pegada-term binary to ~/.local/bin, makes sure an EnergiBridge
# binary is available (it does not assume one is installed), hooks your
# shells, and tests the sensor.
#
# Environment:
#   PEGADA_TERM_YES=1        do not ask before the privileged Linux setup
#   PEGADA_TERM_NO_RC=1      do not touch shell rc files
#   PEGADA_TERM_NO_SETUP=1   skip the privileged Linux setup
#   PEGADA_TERM_VERSION      release tag to install (default: latest)
#   PEGADA_TERM_BIN_DIR      where the binary goes (default: ~/.local/bin)

set -eu

REPO="GreenSeal-dev/pegada-term"
FALLBACK_TAG="v0.1.0"
EB_REPO="tdurieux/EnergiBridge"
EB_FALLBACK_TAG="v0.0.7"
# Overridable so the installer can be tested against a local server.
BASE_URL="${PEGADA_TERM_BASE_URL:-https://github.com/$REPO}"
EB_BASE_URL="${PEGADA_TERM_EB_BASE_URL:-https://github.com/$EB_REPO}"

BIN_DIR="${PEGADA_TERM_BIN_DIR:-$HOME/.local/bin}"
DATA_DIR="${XDG_DATA_HOME:-$HOME/.local/share}/pegada-term"
BEGIN_MARK="# >>> pegada-term >>>"
END_MARK="# <<< pegada-term <<<"

if [ -t 1 ] && [ -z "${NO_COLOR:-}" ]; then
  BOLD=$(printf '\033[1m') DIM=$(printf '\033[2m') GREEN=$(printf '\033[38;5;71m')
  AMBER=$(printf '\033[38;5;214m') RED=$(printf '\033[38;5;203m') RESET=$(printf '\033[0m')
else
  BOLD='' DIM='' GREEN='' AMBER='' RED='' RESET=''
fi

say() { printf '%s\n' "$*"; }
step() { printf '%s==>%s %s\n' "$GREEN" "$RESET" "$*"; }
warn() { printf '%swarning:%s %s\n' "$AMBER" "$RESET" "$*" >&2; }
die() {
  printf '%serror:%s %s\n' "$RED" "$RESET" "$*" >&2
  exit 1
}
have() { command -v "$1" >/dev/null 2>&1; }

# fetch <url> <file>
fetch() {
  if have curl; then
    curl -fsSL --retry 2 -o "$2" "$1"
  elif have wget; then
    wget -q -O "$2" "$1"
  else
    die "curl or wget is required"
  fi
}

# latest_tag <base-url> <fallback>: the tag `releases/latest` redirects to.
# (The GitHub API would also answer this, but it is rate-limited.)
latest_tag() {
  tag=''
  if have curl; then
    url=$(curl -fsSLI -o /dev/null -w '%{url_effective}' "$1/releases/latest" 2>/dev/null) || url=''
    case $url in
      */releases/tag/*) tag=${url##*/} ;;
    esac
  fi
  printf '%s\n' "${tag:-$2}"
}

detect_target() {
  os=$(uname -s)
  arch=$(uname -m)
  case $arch in
    arm64 | aarch64) arch=aarch64 ;;
    x86_64 | amd64) arch=x86_64 ;;
    *) die "unsupported CPU architecture: $arch" ;;
  esac
  case $os in
    Darwin) TARGET="$arch-apple-darwin" ;;
    Linux) TARGET="$arch-unknown-linux-musl" ;;
    *) die "unsupported OS: $os (pegada-term supports macOS and Linux; Windows works through WSL only without a sensor)" ;;
  esac
  OS=$os
}

install_pegada_term() {
  tag=${PEGADA_TERM_VERSION:-$(latest_tag "$BASE_URL" "$FALLBACK_TAG")}
  archive="pegada-term-$tag-$TARGET.tar.gz"
  step "Downloading pegada-term $tag for $TARGET"
  mkdir -p "$BIN_DIR"
  if fetch "$BASE_URL/releases/download/$tag/$archive" "$TMP/$archive" 2>/dev/null; then
    tar -xzf "$TMP/$archive" -C "$TMP"
    [ -f "$TMP/pegada-term" ] || die "the release archive does not contain pegada-term"
    # Copy to a temporary name and rename: safe while an older version is running.
    cp "$TMP/pegada-term" "$BIN_DIR/.pegada-term.new"
    chmod 755 "$BIN_DIR/.pegada-term.new"
    mv -f "$BIN_DIR/.pegada-term.new" "$BIN_DIR/pegada-term"
  elif have cargo; then
    warn "no prebuilt binary for $TARGET at $tag; building with cargo"
    cargo install pegada-term --root "$TMP/cargo" --quiet ||
      cargo install --git "https://github.com/$REPO" --root "$TMP/cargo" --quiet ||
      die "cargo could not build pegada-term"
    cp "$TMP/cargo/bin/pegada-term" "$BIN_DIR/pegada-term"
  else
    die "could not download $archive, and cargo is not installed to build from source"
  fi
  PEGADA_TERM="$BIN_DIR/pegada-term"
  if [ "$OS" = Darwin ]; then
    xattr -d com.apple.quarantine "$PEGADA_TERM" 2>/dev/null || true
  fi
  say "    $PEGADA_TERM"
}

# Sets ENERGIBRIDGE to a working binary, or leaves it empty.
install_energibridge() {
  ENERGIBRIDGE=''
  if have energibridge && energibridge --version >/dev/null 2>&1; then
    ENERGIBRIDGE=$(command -v energibridge)
    step "Using the EnergiBridge already installed: $ENERGIBRIDGE"
    return
  fi
  case $TARGET in
    aarch64-apple-darwin | x86_64-apple-darwin | x86_64-unknown-linux-musl)
      tag=$(latest_tag "$EB_BASE_URL" "$EB_FALLBACK_TAG")
      archive="energibridge-$tag-$TARGET.tar.gz"
      step "Downloading EnergiBridge $tag for $TARGET"
      mkdir -p "$DATA_DIR" "$TMP/eb"
      if ! fetch "$EB_BASE_URL/releases/download/$tag/$archive" "$TMP/$archive" 2>/dev/null; then
        # A new release without this asset: fall back to the pinned one.
        tag=$EB_FALLBACK_TAG
        archive="energibridge-$tag-$TARGET.tar.gz"
        fetch "$EB_BASE_URL/releases/download/$tag/$archive" "$TMP/$archive" ||
          die "could not download EnergiBridge ($archive)"
      fi
      tar -xzf "$TMP/$archive" -C "$TMP/eb"
      found=$(find "$TMP/eb" -type f -name energibridge | head -n 1)
      [ -n "$found" ] || die "the EnergiBridge archive does not contain the binary"
      cp "$found" "$DATA_DIR/energibridge"
      chmod 755 "$DATA_DIR/energibridge"
      ENERGIBRIDGE="$DATA_DIR/energibridge"
      ;;
    *)
      # No prebuilt EnergiBridge for this target (e.g. Linux on ARM).
      if have cargo; then
        step "No EnergiBridge release for $TARGET; building it with cargo (this takes a while)"
        mkdir -p "$DATA_DIR"
        if cargo install --git "https://github.com/$EB_REPO" --root "$TMP/ebcargo" --quiet; then
          cp "$TMP/ebcargo/bin/energibridge" "$DATA_DIR/energibridge"
          ENERGIBRIDGE="$DATA_DIR/energibridge"
        else
          warn "cargo could not build EnergiBridge"
        fi
      else
        warn "there is no EnergiBridge release for $TARGET and cargo is not installed to build it"
      fi
      ;;
  esac
  if [ -n "$ENERGIBRIDGE" ]; then
    if [ "$OS" = Darwin ]; then
      # Downloaded binaries can be quarantined by Gatekeeper.
      xattr -d com.apple.quarantine "$ENERGIBRIDGE" 2>/dev/null || true
    fi
    say "    $ENERGIBRIDGE"
  fi
}

# ask <question>: yes/no from the terminal (stdin is the script under curl | sh).
ask() {
  [ "${PEGADA_TERM_YES:-}" = 1 ] && return 0
  if ! (: </dev/tty) 2>/dev/null; then
    return 1
  fi
  printf '%s [y/N] ' "$1" >/dev/tty
  read -r answer </dev/tty || return 1
  case $answer in
    y | Y | yes | YES) return 0 ;;
    *) return 1 ;;
  esac
}

linux_setup() {
  [ "$OS" = Linux ] || return 0
  [ -n "$ENERGIBRIDGE" ] || return 0
  [ "${PEGADA_TERM_NO_SETUP:-}" = 1 ] && return 0
  if grep -qiE 'microsoft|wsl' /proc/version 2>/dev/null; then
    warn "WSL does not expose MSRs to Linux, so there is no energy sensor here. pegada-term is installed but will show no numbers."
    return 0
  fi
  if [ -x /usr/local/lib/pegada-term/energibridge ] && "$PEGADA_TERM" doctor >/dev/null 2>&1; then
    step "MSR access is already set up"
    return 0
  fi
  step "Privileged setup (Linux)"
  cat <<EOF
    EnergiBridge reads the CPU's RAPL energy counters through /dev/cpu/*/msr,
    which only root can read. With sudo, \`pegada-term setup\` will:
      - create the group ${BOLD}msr${RESET}
      - install EnergiBridge as /usr/local/lib/pegada-term/energibridge,
        root-owned, setgid msr, with the capability cap_sys_rawio
      - add a udev rule making /dev/cpu/*/msr readable by group msr
      - load the msr module now and at boot (/etc/modules-load.d)
    No re-login is needed, and \`pegada-term uninstall\` reverses it.

    ${AMBER}Security trade-off:${RESET} this lets every local user read RAPL through
    EnergiBridge. RAPL is a known power side channel (PLATYPUS, CVE-2020-8694),
    which is why Linux keeps it root-only. Do not do this on a shared machine.
EOF
  if ask "    Run \`sudo pegada-term setup\` now?"; then
    sudo "$PEGADA_TERM" setup --energibridge "$ENERGIBRIDGE" ||
      warn "setup failed; run \`pegada-term doctor\` for details"
  else
    say "    Skipped. Until you run it, pegada-term shows no numbers:"
    say "      sudo $PEGADA_TERM setup --energibridge $ENERGIBRIDGE"
  fi
}

# add_block <rc-file> <shell>
add_block() {
  rc=$1
  if [ -f "$rc" ] && grep -qF "$BEGIN_MARK" "$rc"; then
    say "    $rc (already hooked)"
    return
  fi
  mkdir -p "$(dirname "$rc")"
  # Exactly this text is removed again by `pegada-term uninstall`.
  # shellcheck disable=SC2016
  printf '\n%s\n[ -x "%s" ] && eval "$("%s" init %s)"\n%s\n' \
    "$BEGIN_MARK" "$PEGADA_TERM" "$PEGADA_TERM" "$2" "$END_MARK" >>"$rc"
  say "    $rc"
}

hook_shells() {
  [ "${PEGADA_TERM_NO_RC:-}" = 1 ] && return 0
  step "Adding the hook to your shells"
  if have zsh || [ -f "${ZDOTDIR:-$HOME}/.zshrc" ]; then
    add_block "${ZDOTDIR:-$HOME}/.zshrc" zsh
  fi
  if have bash; then
    # macOS terminals start login shells, which read .bash_profile, not .bashrc.
    if [ "$OS" = Darwin ] && [ -f "$HOME/.bash_profile" ] && [ ! -f "$HOME/.bashrc" ]; then
      add_block "$HOME/.bash_profile" bash
    else
      add_block "$HOME/.bashrc" bash
    fi
  fi
  if have fish; then
    conf="${XDG_CONFIG_HOME:-$HOME/.config}/fish/conf.d/pegada-term.fish"
    mkdir -p "$(dirname "$conf")"
    printf '# Added by the pegada-term installer.\ntest -x "%s"; and "%s" init fish | source\n' \
      "$PEGADA_TERM" "$PEGADA_TERM" >"$conf"
    say "    $conf"
  fi
}

smoke_test() {
  step "Testing the sensor"
  if result=$("$PEGADA_TERM" doctor --smoke 2>&1); then
    SENSOR_OK=1
    say "    ${GREEN}$result${RESET}"
  else
    SENSOR_OK=0
    say "    ${AMBER}$result${RESET}"
  fi
}

banner() {
  say ""
  say "  ${BOLD}⚡ pegada-term $("$PEGADA_TERM" version | cut -d' ' -f2) is installed${RESET}"
  say ""
  say "  Open a new terminal and run something. After each command you get:"
  say ""
  say "    ${DIM}⚡ 142 J total · ${RESET}${GREEN}38 J${RESET}${DIM} above idle · 12.4 s · 11.5 W  ${RESET}${GREEN}▂▃${RESET}${AMBER}▅▇${RESET}${RED}█${RESET}${AMBER}▇▅${RESET}${GREEN}▃▂${RESET}${DIM}  · session 3.2 kJ${RESET}"
  say ""
  say "    pegada-term status   sampler, sensor, live power, totals"
  say "    pegada-term stats    top commands by energy"
  say "    pegada-term watch    live power meter"
  say "    pegada-term doctor   when something looks wrong"
  if [ "$SENSOR_OK" != 1 ]; then
    say ""
    say "  ${AMBER}The sensor is not working yet.${RESET} Run: $PEGADA_TERM doctor"
  fi
  case ":$PATH:" in
    *":$BIN_DIR:"*) ;;
    *)
      say ""
      say "  ${DIM}$BIN_DIR is not on your PATH. New shells get a \`pegada-term\` function from${RESET}"
      say "  ${DIM}the hook, so the command works anyway.${RESET}"
      ;;
  esac
  say ""
}

main() {
  TMP=$(mktemp -d "${TMPDIR:-/tmp}/pegada-term-install.XXXXXX")
  trap 'rm -rf "$TMP"' EXIT
  detect_target
  install_pegada_term
  install_energibridge
  linux_setup
  hook_shells
  smoke_test
  banner
}

# Everything is inside functions so a truncated download cannot run half a script.
main "$@"
