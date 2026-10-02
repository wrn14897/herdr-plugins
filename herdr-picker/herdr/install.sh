#!/usr/bin/env bash
# herdr `[[build]]` step: put a herdr-picker binary at bin/herdr-picker.
#
# 1. Download the prebuilt binary for this platform from the GitHub release
#    tagged herdr-picker-v<manifest version>, verifying its sha256.
# 2. If that is not possible, build from source with cargo (Rust 1.85+).
#
# `herdr plugin link` does not run build steps: after linking a checkout, run
# this script once yourself. Set HERDR_PICKER_FROM_SOURCE=1 to skip the download.
#
# Build commands run with the plugin checkout as the working directory and may
# not receive runtime plugin env, so paths resolve from this script's location.
set -euo pipefail

NAME="herdr-picker"
REPO="${HERDR_PICKER_REPO:-wrn14897/herdr-plugins}"
MIN_RUST_MINOR=85 # edition 2024 needs Rust 1.85

ROOT="$(cd "$(dirname "$0")/.." && pwd)"
BIN_DIR="$ROOT/bin"
VERSION="$(grep -m1 '^version' "$ROOT/herdr-plugin.toml" | sed -E 's/.*"([^"]+)".*/\1/')"
TAG="${NAME}-v${VERSION}"

log() { echo "$NAME: $*" >&2; }

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

# Downloads into $1 (a scratch dir). Every failure says why.
download_into() {
  local tmp="$1" target archive base expected actual
  if ! target="$(target_triple)"; then
    log "no prebuilt binary for $(uname -s) $(uname -m)"
    return 1
  fi
  if ! command -v curl >/dev/null 2>&1; then
    log "curl is not installed"
    return 1
  fi

  archive="${NAME}-${target}.tar.gz"
  base="https://github.com/${REPO}/releases/download/${TAG}"
  log "downloading $archive ($TAG)"
  if ! curl -fsSL --retry 3 --retry-delay 2 "$base/$archive" -o "$tmp/$archive" ||
    ! curl -fsSL --retry 3 --retry-delay 2 "$base/$archive.sha256" -o "$tmp/$archive.sha256"; then
    log "could not download $base/$archive (offline, blocked, or not released yet)"
    return 1
  fi

  expected="$(awk '{print $1}' "$tmp/$archive.sha256")"
  actual="$(sha256 "$tmp/$archive")"
  if [ "$expected" != "$actual" ]; then
    log "checksum mismatch (expected $expected, got $actual)"
    return 1
  fi

  tar -xzf "$tmp/$archive" -C "$tmp"
  mkdir -p "$BIN_DIR"
  install -m 0755 "$tmp/$NAME" "$BIN_DIR/$NAME"
}

download() {
  local tmp status=0
  tmp="$(mktemp -d)"
  download_into "$tmp" || status=$?
  rm -rf "$tmp"
  return "$status"
}

# Prints the minor version of a `cargo` command (1.85.0 -> 85), or nothing.
rust_minor() {
  "$@" --version 2>/dev/null | sed -nE 's/^cargo 1\.([0-9]+).*/\1/p'
}

# Sets CARGO (+ CARGO_TOOLCHAIN) to a cargo new enough for this crate. Plugin
# builds may not inherit an interactive PATH, so common install locations are
# checked too. A rustup "stable" toolchain is used over an older default.
# Returns 1 when no cargo exists, 2 when every cargo is too old.
CARGO=""
CARGO_TOOLCHAIN=()
find_cargo() {
  local candidate minor
  local -a candidates=()
  command -v cargo >/dev/null 2>&1 && candidates+=("$(command -v cargo)")
  for candidate in "$HOME/.cargo/bin/cargo" /opt/homebrew/opt/rustup/bin/cargo /usr/local/opt/rustup/bin/cargo; do
    [ -x "$candidate" ] && candidates+=("$candidate")
  done
  [ "${#candidates[@]}" -gt 0 ] || return 1

  for candidate in "${candidates[@]}"; do
    minor="$(rust_minor "$candidate")"
    if [ -n "$minor" ] && [ "$minor" -ge "$MIN_RUST_MINOR" ]; then
      CARGO="$candidate"
      return 0
    fi
    # rustup proxies accept a toolchain override.
    minor="$(rust_minor "$candidate" +stable)"
    if [ -n "$minor" ] && [ "$minor" -ge "$MIN_RUST_MINOR" ]; then
      CARGO="$candidate"
      CARGO_TOOLCHAIN=(+stable)
      return 0
    fi
  done

  log "found cargo 1.$(rust_minor "${candidates[0]}"), but building needs Rust 1.$MIN_RUST_MINOR or newer"
  log "update it with: rustup update stable"
  return 2
}

build_from_source() {
  local status=0
  find_cargo || status=$?
  if [ "$status" -eq 1 ]; then
    log "cargo is not installed; install Rust 1.$MIN_RUST_MINOR+ from https://rustup.rs and run this script again"
  fi
  [ "$status" -eq 0 ] || return 1

  log "building from source with $CARGO ${CARGO_TOOLCHAIN[*]:-}"
  # The toolchain (rustc, ...) lives next to cargo; plugin builds may lack it on PATH.
  (cd "$ROOT" && PATH="$(dirname "$CARGO"):$PATH" "$CARGO" ${CARGO_TOOLCHAIN[@]+"${CARGO_TOOLCHAIN[@]}"} build --release --locked)
  mkdir -p "$BIN_DIR"
  install -m 0755 "$ROOT/target/release/$NAME" "$BIN_DIR/$NAME"
}

if [ "${HERDR_PICKER_FROM_SOURCE:-0}" = 1 ]; then
  build_from_source
elif ! download; then
  log "falling back to building from source"
  build_from_source
fi
log "installed $BIN_DIR/$NAME"
