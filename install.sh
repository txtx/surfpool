#!/bin/bash

set -euo pipefail

###############################################
#                 PLATFORM DETECT             #
###############################################

OS="$(uname)"
case "$OS" in
  Darwin)  ON_MACOS=1 ;;
  Linux)   ON_LINUX=1 ;;
  *) echo "surfpool is only supported on macOS and Linux." >&2; exit 1 ;;
esac

PRODUCT="surfpool"
GITHUB_REPO="txtx/surfpool"
VERSION="${VERSION:-latest}"
if [[ "$VERSION" == "latest" ]]; then
  BASE_URL="https://github.com/${GITHUB_REPO}/releases/latest/download"
else
  BASE_URL="https://github.com/${GITHUB_REPO}/releases/download/${VERSION}"
fi

# CPU architecture → correct tarball
if [[ -n "${ON_MACOS-}" ]]; then
  ARCH="$(uname -m)"
  if [[ "$ARCH" == "arm64" ]]; then
    TAR_URL="${BASE_URL}/${PRODUCT}-darwin-arm64.tar.gz"
  else
    TAR_URL="${BASE_URL}/${PRODUCT}-darwin-x64.tar.gz"
  fi
else
  TAR_URL="${BASE_URL}/${PRODUCT}-linux-x64.tar.gz"
fi

###############################################
#            INSTALL DIRECTORY CHOICE         #
###############################################

# Prefer ~/.local/bin; fallback to ~/bin
if mkdir -p "$HOME/.local/bin" 2>/dev/null; then
  INSTALL_DIR="$HOME/.local/bin"
else
  mkdir -p "$HOME/bin"
  INSTALL_DIR="$HOME/bin"
fi

DST="${INSTALL_DIR}/${PRODUCT}"

###############################################
#                    COLORS                   #
###############################################

if [[ -t 1 ]]; then
  tty_escape() { printf "\033[%sm" "$1"; }
else
  tty_escape() { :; }
fi

tty_mkbold() { tty_escape "1;$1"; }

tty_blue="$(tty_mkbold 34)"
tty_green="$(tty_mkbold 32)"
tty_orange="$(tty_mkbold 33)"
tty_bold="$(tty_mkbold 39)"
tty_reset="$(tty_escape 0)"

###############################################
#                   HELPERS                   #
###############################################

shell_join() {
  local arg; printf "%s" "$1"; shift
  for arg in "$@"; do printf " %s" "${arg// /\ }"; done
}

abort() {
  printf "%s\n" "$@" >&2
  exit 1
}

ohai() {
  printf "${tty_orange}→${tty_bold} %s${tty_reset}\n" "$(shell_join "$@")"
}

###############################################
#                  CLEANUP                    #
###############################################

CLEAN_FILES=(
  "${PRODUCT}.tar.gz"
  "${PRODUCT}.tar"
  "${PRODUCT}"
)

cleanup() {
  for f in "${CLEAN_FILES[@]}"; do
    [[ -f "$f" ]] && rm -f "$f"
  done
}

trap cleanup EXIT

###############################################
#            DOWNLOAD + EXTRACT               #
###############################################

echo ""
ohai "Downloading and installing ${tty_green}${PRODUCT}${tty_reset} from ${tty_orange}${TAR_URL}${tty_reset}"

curl -s -o "${PRODUCT}.tar.gz" -L "$TAR_URL"
gzip -d "${PRODUCT}.tar.gz"
tar -xf "${PRODUCT}.tar"

# macOS quarantine attribute
if [[ -n "${ON_MACOS-}" ]]; then
  xattr -d com.apple.quarantine "./${PRODUCT}" 2>/dev/null || true
fi

###############################################
#                  INSTALL                    #
###############################################

ohai "Installing ${DST}"
install -m 0755 "${PRODUCT}" "${DST}"

echo -e "${tty_green}✓${tty_reset} Install successful"

###############################################
#           AUTO-ADD TO PATH (SAFE)           #
###############################################

add_path_if_needed() {
  TARGET_DIR="$1"
  SHELL_RC=""

  # Detect user's shell rc file
  case "$SHELL" in
    */zsh)  SHELL_RC="$HOME/.zshrc" ;;
    */bash) SHELL_RC="$HOME/.bashrc" ;;
    */fish) SHELL_RC="$HOME/.config/fish/config.fish" ;;
    *)      SHELL_RC="$HOME/.profile" ;;
  esac

  mkdir -p "$(dirname "$SHELL_RC")"
  touch "$SHELL_RC"

  # Only add if missing
  if ! grep -q "$TARGET_DIR" "$SHELL_RC"; then
    echo "export PATH=\"$TARGET_DIR:\$PATH\"" >> "$SHELL_RC"
    echo "Added $TARGET_DIR to PATH in $SHELL_RC"
  fi
}

if ! command -v "$PRODUCT" >/dev/null 2>&1; then
  echo ""
  ohai "Adding ${INSTALL_DIR} to your PATH"
  add_path_if_needed "$INSTALL_DIR"
  echo ""
  echo "${tty_green}✓${tty_reset} PATH updated"
  echo "   Restart your terminal or run:"
  echo "     source ~/.${SHELL##*/}rc"
  echo ""
fi

###############################################
#                   EPILOG                    #
###############################################

cat <<EOS
${tty_blue}+-----------------+
|                 |
|                 |
|   ##            |
|  #####  ##  ##  |
|   ##     # ##   |
|   ##  #  ## #   |
|    ###  ##   #  |
|                 |
|                 |
+-----------------+
${tty_orange}→${tty_reset} Run ${tty_green}${PRODUCT} start${tty_reset} to get started
${tty_orange}→${tty_reset} Further documentation: ${tty_orange}https://docs.${PRODUCT}.run${tty_reset}
EOS
