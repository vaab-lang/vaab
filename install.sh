#!/bin/sh
# Install the Vaab CLI.
#
#   curl -fsSL https://raw.githubusercontent.com/vaab-lang/vaab/master/install.sh | sh
#
# Options via env:
#   VAAB_VERSION      Release tag to install (default: latest)
#   VAAB_INSTALL_DIR  Where to put the binary (default: ~/.local/bin)

set -eu

REPO="vaab-lang/vaab"
VERSION="${VAAB_VERSION:-latest}"
INSTALL_DIR="${VAAB_INSTALL_DIR:-${HOME}/.local/bin}"

say() {
  printf 'vaab-install: %s\n' "$*"
}

err() {
  printf 'vaab-install: %s\n' "$*" >&2
  exit 1
}

need_cmd() {
  command -v "$1" >/dev/null 2>&1 || err "need '$1' (command not found)"
}

download() {
  url="$1"
  dest="$2"
  if command -v curl >/dev/null 2>&1; then
    curl -fsSL "$url" -o "$dest"
  elif command -v wget >/dev/null 2>&1; then
    wget -qO "$dest" "$url"
  else
    err "need 'curl' or 'wget'"
  fi
}

detect_target() {
  os="$(uname -s)"
  arch="$(uname -m)"

  case "$os" in
    Linux)  os_name="linux" ;;
    Darwin) os_name="macos" ;;
    *)      err "unsupported OS: $os (need Linux or macOS)" ;;
  esac

  case "$arch" in
    x86_64 | amd64)  arch_name="x86_64" ;;
    aarch64 | arm64) arch_name="aarch64" ;;
    *)               err "unsupported architecture: $arch" ;;
  esac

  # Published release assets today: linux-x86_64, macos-aarch64.
  case "${os_name}-${arch_name}" in
    linux-x86_64 | macos-aarch64) ;;
    macos-x86_64)
      err "no prebuilt binary for macOS Intel yet; build from source with: cargo install --git https://github.com/${REPO} --locked vaab-cli"
      ;;
    linux-aarch64)
      err "no prebuilt binary for Linux ARM yet; build from source with: cargo install --git https://github.com/${REPO} --locked vaab-cli"
      ;;
    *)
      err "no prebuilt binary for ${os_name}-${arch_name}"
      ;;
  esac

  printf 'vaab-%s-%s' "$os_name" "$arch_name"
}

main() {
  need_cmd uname
  need_cmd mktemp
  need_cmd mkdir
  need_cmd chmod
  need_cmd mv

  target="$(detect_target)"
  if [ "$VERSION" = "latest" ]; then
    url="https://github.com/${REPO}/releases/latest/download/${target}"
  else
    url="https://github.com/${REPO}/releases/download/${VERSION}/${target}"
  fi

  tmp="$(mktemp -d)"
  trap 'rm -rf "$tmp"' EXIT

  say "downloading ${target} (${VERSION})"
  if ! download "$url" "${tmp}/vaab"; then
    err "download failed: ${url}"
  fi

  mkdir -p "$INSTALL_DIR"
  chmod +x "${tmp}/vaab"
  mv "${tmp}/vaab" "${INSTALL_DIR}/vaab"

  say "installed ${INSTALL_DIR}/vaab"
  if ! command -v vaab >/dev/null 2>&1; then
    say "add this to your PATH, then open a new shell:"
    say "  export PATH=\"${INSTALL_DIR}:\$PATH\""
  fi

  if "${INSTALL_DIR}/vaab" version >/dev/null 2>&1; then
    say "ok — try: vaab run main.vaab"
  else
    say "binary installed; run: ${INSTALL_DIR}/vaab version"
  fi
}

main
