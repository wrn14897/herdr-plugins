#!/usr/bin/env bash
# herdr `[[build]]` step: put a herdr-picker binary at bin/herdr-picker.
#
# 1. Download the prebuilt binary for this platform from the GitHub release
#    tagged herdr-picker-v<manifest version>, verifying its sha256.
# 2. If no release asset is available, build from source with cargo.
#
# Build commands run with the plugin checkout as the working directory and may
# not receive runtime plugin env, so paths resolve from this script's location.
set -euo pipefail

NAME="herdr-picker"
REPO="${HERDR_PICKER_REPO:-wrn14897/herdr-plugins}"

ROOT="$(cd "$(dirname "$0")/.." && pwd)"
BIN_DIR="$ROOT/bin"
VERSION="$(grep -m1 '^version' "$ROOT/herdr-plugin.toml" | sed -E 's/.*"([^"]+)".*/\1/')"
TAG="${NAME}-v${VERSION}"

log() { echo "$NAME: $*"; }

target_triple() {
  case "$(uname -s)-$(uname -m)" in
    Darwin-arm64) echo "aarch64-apple-darwin" ;;
    Darwin-x86_64) echo "x86_64-apple-darwin" ;;
    Linux-aarch64 | Linux-arm64) echo "aarch64-unknown-linux-musl" ;;
    Linux-x86_64) echo "x86_64-unknown-linux-musl" ;;
    *) return 1 ;;
  esac
}

sha256() {
  if command -v sha256sum >/dev/null 2>&1; then
    sha256sum "$1" | awk '{print $1}'
  else
    shasum -a 256 "$1" | awk '{print $1}'
  fi
}

download() {
  local target archive base tmp expected actual
  target="$(target_triple)" || { log "no prebuilt binary for $(uname -s)-$(uname -m)"; return 1; }
  command -v curl >/dev/null 2>&1 || { log "curl not found"; return 1; }

  archive="${NAME}-${target}.tar.gz"
  base="https://github.com/${REPO}/releases/download/${TAG}"
  tmp="$(mktemp -d)"
  trap 'rm -rf "$tmp"' RETURN

  log "downloading $archive ($TAG)"
  curl -fsSL --retry 3 --retry-delay 2 "$base/$archive" -o "$tmp/$archive" || return 1
  curl -fsSL --retry 3 --retry-delay 2 "$base/$archive.sha256" -o "$tmp/$archive.sha256" || return 1

  expected="$(awk '{print $1}' "$tmp/$archive.sha256")"
  actual="$(sha256 "$tmp/$archive")"
  if [ "$expected" != "$actual" ]; then
    log "checksum mismatch (expected $expected, got $actual)" >&2
    return 1
  fi

  tar -xzf "$tmp/$archive" -C "$tmp"
  mkdir -p "$BIN_DIR"
  install -m 0755 "$tmp/$NAME" "$BIN_DIR/$NAME"
}

build_from_source() {
  local cargo
  cargo="$(command -v cargo || true)"
  [ -n "$cargo" ] || [ ! -x "$HOME/.cargo/bin/cargo" ] || cargo="$HOME/.cargo/bin/cargo"
  if [ -z "$cargo" ]; then
    log "no release asset and cargo is not installed; install Rust (https://rustup.rs) and reinstall" >&2
    return 1
  fi
  log "building from source with $cargo"
  (cd "$ROOT" && "$cargo" build --release --locked)
  mkdir -p "$BIN_DIR"
  install -m 0755 "$ROOT/target/release/$NAME" "$BIN_DIR/$NAME"
}

if [ "${HERDR_PICKER_FROM_SOURCE:-0}" != 1 ] && download; then
  :
else
  build_from_source
fi
log "installed $BIN_DIR/$NAME"
