#!/bin/sh
# Cairn installer
#
# Usage (latest):
#   curl -fsSL https://raw.githubusercontent.com/towerforge/cairn/main/install.sh | sh
#
# What it installs:
#   macOS  Cairn.app in /Applications (~/Applications if that is not writable)
#          and a `cairn` link in ~/.local/bin for `cairn update`
#   Linux  cairn in ~/.local/bin, plus the launcher and icon in ~/.local/share
#          (/usr/local when run as root)
#
# Env overrides:
#   CAIRN_VERSION=0.4.0         install a specific version
#   CAIRN_INSTALL_DIR=/opt/bin  where the `cairn` command goes
#   CAIRN_APP_DIR=~/Apps        macOS: where Cairn.app goes
#   CAIRN_FORCE=1               install even if Cairn is already there and can
#                               update itself (`cairn update`)
#   NO_COLOR=1                  no colour, whatever the terminal says

set -e

REPO="towerforge/cairn"
BINARY="cairn"
GITHUB_API="https://api.github.com/repos/${REPO}"
GITHUB_RELEASES="https://github.com/${REPO}/releases/download"

# ── the Cairn palette ────────────────────────────────────────────────────────
# The window's accent (#43B18D) and greys: the full hex when the terminal
# announces truecolor, the xterm-256 approximation otherwise, and nothing at
# all when the output is not a terminal.

if [ -t 1 ] && [ -z "${NO_COLOR:-}" ] && [ "${TERM:-}" != "dumb" ]; then
  BOLD='\033[1m'; DIM='\033[2m'; RESET='\033[0m'
  case "${COLORTERM:-}" in
    truecolor|24bit)
      ACCENT='\033[38;2;67;177;141m'  # accent  #43b18d
      INK='\033[38;2;230;230;230m'    # text    #e6e6e6
      MUTED='\033[38;2;139;147;163m'  # subtle  #8b93a3
      LINE='\033[38;2;90;90;90m'      # dim     #5a5a5a
      GREEN='\033[38;2;152;195;121m'  # ok      #98c379
      RED='\033[38;2;224;108;117m'    # error   #e06c75
      ;;
    *)
      ACCENT='\033[38;5;72m'; INK='\033[38;5;254m'; MUTED='\033[38;5;103m'
      LINE='\033[38;5;240m'; GREEN='\033[38;5;114m'; RED='\033[38;5;168m'
      ;;
  esac
else
  BOLD=''; DIM=''; RESET=''
  ACCENT=''; INK=''; MUTED=''; LINE=''; GREEN=''; RED=''
fi

# ── the pieces every block is drawn with ─────────────────────────────────────

# Columns a rule spans, the two-space indent aside.
RULE_W=66
# Column the descriptions of a command list line up at.
CMD_W=24

_dashes() {
  _n=$1; _s=''
  while [ "$_n" -gt 0 ]; do _s="${_s}┄"; _n=$((_n - 1)); done
  printf '%s' "$_s"
}

# Section heading: `┄ 2 · release ┄┄┄…`, the title in the accent over a
# dashed rule. The number is optional. Everything is ASCII, so bytes count
# as columns.
rule() {
  if [ -n "$1" ]; then
    _label="$1 · $2"; _cols=$(( ${#1} + 3 + ${#2} ))
  else
    _label="$2"; _cols=${#2}
  fi
  _fill=$(( RULE_W - _cols - 3 ))
  [ "$_fill" -lt 0 ] && _fill=0
  printf "\n  ${LINE}┄${RESET} ${ACCENT}${BOLD}%s${RESET} ${LINE}%s${RESET}\n" \
    "$_label" "$(_dashes "$_fill")"
}

kv()   { printf "     ${MUTED}%-13s${RESET}${INK}%b${RESET}\n" "$1" "$2"; }
note() { printf "     ${MUTED}%s${RESET}\n" "$*"; }
cmd()  { printf "     ${ACCENT}%-${CMD_W}s${RESET}${MUTED}%s${RESET}\n" "$1" "$2"; }
ok()   { printf "     ${GREEN}✓${RESET} %b\n" "$*"; }
warn() { printf "     ${RED}!${RESET} ${MUTED}%s${RESET}\n" "$*"; }
die()  { printf "\n  ${RED}✗${RESET} %s\n\n" "$*" >&2; exit 1; }

# ── requirements ─────────────────────────────────────────────────────────────

need() { command -v "$1" >/dev/null 2>&1 || die "Required tool not found: $1"; }
need curl
need tar

# ── platform detection ───────────────────────────────────────────────────────

detect_os() {
  case "$(uname -s)" in
    Linux)  echo linux ;;
    Darwin) echo macos ;;
    *)      die "Unsupported OS: $(uname -s). Cairn runs on macOS and Linux." ;;
  esac
}

detect_arch() {
  case "$(uname -m)" in
    x86_64|amd64)  echo x86_64  ;;
    aarch64|arm64) echo aarch64 ;;
    *)             die "No prebuilt Cairn for $(uname -m). Build from source: cargo install --git https://github.com/${REPO}" ;;
  esac
}

# ── github helpers ───────────────────────────────────────────────────────────

fetch_latest_version() {
  curl -fsSL "${GITHUB_API}/releases/latest" \
    | grep '"tag_name"' \
    | sed 's/.*"tag_name": *"v\([^"]*\)".*/\1/'
}

# ── checksum verification ────────────────────────────────────────────────────

verify_checksum() {
  _file="$1"; _sums="$2"
  _name=$(basename "$_file")
  _expected=$(grep " ${_name}$" "$_sums" 2>/dev/null | awk '{print $1}')

  if [ -z "$_expected" ]; then
    warn "no checksum entry for ${_name}: not verified"
    return 0
  fi

  if command -v sha256sum >/dev/null 2>&1; then
    _actual=$(sha256sum "$_file" | awk '{print $1}')
  elif command -v shasum >/dev/null 2>&1; then
    _actual=$(shasum -a 256 "$_file" | awk '{print $1}')
  else
    warn "sha256sum / shasum not found: not verified"
    return 0
  fi

  [ "$_actual" = "$_expected" ] \
    || die "Checksum mismatch!
  expected: ${_expected}
  got:      ${_actual}"

  ok "sha-256 verified"
}

# ── install locations ────────────────────────────────────────────────────────

is_root() { [ "$(id -u)" = "0" ]; }

default_bin_dir() {
  if is_root; then echo /usr/local/bin; else echo "${HOME}/.local/bin"; fi
}

default_share_dir() {
  if is_root; then echo /usr/local/share; else echo "${HOME}/.local/share"; fi
}

# /Applications when we may write there (admins can), ~/Applications otherwise.
default_app_dir() {
  if [ -w /Applications ]; then echo /Applications; else echo "${HOME}/Applications"; fi
}

# ─────────────────────────────────────────────────────────────────────────────
# MAIN
# ─────────────────────────────────────────────────────────────────────────────

printf "\n"
printf "  ${ACCENT}${BOLD}◆ cairn${RESET}   ${MUTED}a block-based terminal for macOS and Linux${RESET}\n"
printf "            ${LINE}installer · github.com/${REPO}${RESET}\n"

# ── step 1: detect platform ──────────────────────────────────────────────────

rule 1 platform

OS=$(detect_os)
ARCH=$(detect_arch)
kv "os" "$OS"
kv "arch" "$ARCH"

# ── step 2: resolve version ──────────────────────────────────────────────────

rule 2 release

BIN_DIR="${CAIRN_INSTALL_DIR:-$(default_bin_dir)}"
APP_DIR=""
[ "$OS" = "macos" ] && APP_DIR="${CAIRN_APP_DIR:-$(default_app_dir)}"

# Detect an existing installation
CURRENT_VERSION=""
EXISTING_PATH=""
for _candidate in "${BIN_DIR}/${BINARY}" "${APP_DIR:+${APP_DIR}/Cairn.app/Contents/MacOS/${BINARY}}" "$(command -v ${BINARY} 2>/dev/null || true)"; do
  [ -z "$_candidate" ] && continue
  if [ -x "$_candidate" ]; then
    EXISTING_PATH="$_candidate"
    CURRENT_VERSION=$("$_candidate" --version 2>/dev/null | awk '{print $NF}' || true)
    break
  fi
done

# Cairn updates itself, so this script is for the first install. Whether the
# Cairn that is there can do it is asked of the binary and not of its version
# number: one from before `cairn update` existed is upgraded here as always.
if [ -n "$CURRENT_VERSION" ] && [ -z "${CAIRN_FORCE:-}" ] \
   && "$EXISTING_PATH" update --help >/dev/null 2>&1; then
  kv "version" "v${CURRENT_VERSION}"
  kv "path" "${EXISTING_PATH}"
  ok "Cairn is already installed"
  printf "\n"
  printf "     ${INK}It updates itself.${RESET} ${MUTED}From here on:${RESET}\n"
  printf "\n"
  cmd "/update" "inside Cairn, when it tells you there is one"
  cmd "cairn update" "the latest release, from a terminal"
  cmd "cairn update --check" "is there a new one?"
  cmd "cairn update --to 0.4.0" "that version, downgrades included"
  printf "\n"
  note "to install with this script anyway:"
  note "curl -fsSL https://raw.githubusercontent.com/${REPO}/main/install.sh | CAIRN_FORCE=1 sh"
  printf "\n"
  exit 0
fi

VERSION="${CAIRN_VERSION:-}"
if [ -z "$VERSION" ]; then
  note "asking github for the latest release…"
  VERSION=$(fetch_latest_version) || die "Could not fetch the latest version from GitHub"
  [ -n "$VERSION" ] || die "No release found at https://github.com/${REPO}/releases"
fi

PACKAGE="${BINARY}-${OS}-${ARCH}.tar.gz"
URL="${GITHUB_RELEASES}/v${VERSION}/${PACKAGE}"
CHECKSUMS_URL="${GITHUB_RELEASES}/v${VERSION}/checksums.txt"

if [ -z "$CURRENT_VERSION" ]; then
  MODE="install"
  kv "version" "v${VERSION}"
elif [ "$CURRENT_VERSION" = "$VERSION" ]; then
  MODE="reinstall"
  kv "version" "v${VERSION}  ${DIM}(already installed)${RESET}"
else
  MODE="upgrade"
  kv "version" "${MUTED}v${CURRENT_VERSION}${RESET}  ${LINE}→${RESET}  ${ACCENT}v${VERSION}${RESET}"
fi
kv "package" "$PACKAGE"
if [ "$OS" = "macos" ]; then
  kv "app" "${APP_DIR}/Cairn.app"
fi
kv "command" "${BIN_DIR}/${BINARY}"

# ── confirmation ─────────────────────────────────────────────────────────────

if [ -t 0 ] || [ -c /dev/tty ]; then
  printf "\n"
  if [ "$MODE" = "reinstall" ]; then
    printf "     ${MUTED}already at v${VERSION} · reinstall?${RESET} ${INK}[y/N]${RESET} "
    read -r _reply </dev/tty
    case "$_reply" in
      [yY]*) ;;
      *) printf "\n     ${MUTED}nothing was touched${RESET}\n\n"; exit 0 ;;
    esac
  else
    _action="install"
    [ "$MODE" = "upgrade" ] && _action="upgrade"
    printf "     ${MUTED}press${RESET} ${INK}enter${RESET} ${MUTED}to ${_action}, or${RESET} ${INK}ctrl+c${RESET} ${MUTED}to cancel${RESET} "
    read -r _ </dev/tty
  fi
fi

# ── step 3: download & verify ────────────────────────────────────────────────

rule 3 download

TMP=$(mktemp -d)
trap 'rm -rf "$TMP"' EXIT INT TERM

note "${URL}"
curl -fL --progress-bar -o "${TMP}/${PACKAGE}" "$URL" \
  || die "Download failed. Check that release v${VERSION} has the asset ${PACKAGE}:
  https://github.com/${REPO}/releases/tag/v${VERSION}"

if curl -fsSL -o "${TMP}/checksums.txt" "$CHECKSUMS_URL" 2>/dev/null; then
  verify_checksum "${TMP}/${PACKAGE}" "${TMP}/checksums.txt"
else
  warn "checksums.txt not published for v${VERSION}: not verified"
fi

# ── step 4: install ──────────────────────────────────────────────────────────

rule 4 install

mkdir -p "${TMP}/x"
tar -xzf "${TMP}/${PACKAGE}" -C "${TMP}/x"
mkdir -p "$BIN_DIR"

if [ "$OS" = "macos" ]; then
  [ -x "${TMP}/x/Cairn.app/Contents/MacOS/${BINARY}" ] || die "Cairn.app not found inside the archive"
  mkdir -p "$APP_DIR"
  rm -rf "${APP_DIR}/Cairn.app"
  mv "${TMP}/x/Cairn.app" "${APP_DIR}/Cairn.app"
  # curl does not quarantine what it downloads, but a copy made by hand might.
  xattr -dr com.apple.quarantine "${APP_DIR}/Cairn.app" 2>/dev/null || true
  ln -sf "${APP_DIR}/Cairn.app/Contents/MacOS/${BINARY}" "${BIN_DIR}/${BINARY}"
  ok "${APP_DIR}/Cairn.app  ${DIM}(v${VERSION})${RESET}"
  ok "${BIN_DIR}/${BINARY}  ${DIM}→ Cairn.app${RESET}"
else
  [ -f "${TMP}/x/${BINARY}" ] || die "Binary '${BINARY}' not found inside the archive"
  SHARE_DIR="$(default_share_dir)"
  install -m 755 "${TMP}/x/${BINARY}" "${BIN_DIR}/${BINARY}"
  mkdir -p "${SHARE_DIR}/applications" "${SHARE_DIR}/icons/hicolor/128x128/apps"
  install -m 644 "${TMP}/x/share/applications/cairn.desktop" "${SHARE_DIR}/applications/cairn.desktop"
  install -m 644 "${TMP}/x/share/icons/hicolor/128x128/apps/cairn.png" "${SHARE_DIR}/icons/hicolor/128x128/apps/cairn.png"
  command -v update-desktop-database >/dev/null 2>&1 \
    && update-desktop-database "${SHARE_DIR}/applications" 2>/dev/null || true
  ok "${BIN_DIR}/${BINARY}  ${DIM}(v${VERSION})${RESET}"
  ok "launcher in ${SHARE_DIR}/applications"
fi

# PATH hint
case ":${PATH}:" in
  *":${BIN_DIR}:"*) ;;
  *)
    warn "${BIN_DIR} is not in your PATH"
    note "add to your shell profile:  export PATH=\"${BIN_DIR}:\$PATH\""
    ;;
esac

# ── done ─────────────────────────────────────────────────────────────────────

rule "" ready

if [ "$OS" = "macos" ]; then
  cmd "open -a Cairn" "the window (or from Launchpad / Spotlight)"
else
  cmd "cairn" "the window (or from the applications menu)"
fi
cmd "cairn update" "when there is a new release"
printf "\n"
note "Inside Cairn, Ctrl+G shows every shortcut and /help every command."
printf "\n"
