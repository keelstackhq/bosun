#!/usr/bin/env bash
# Setup script for the Claude Code cloud environment of bosun and of repos that use it
# (spec D23). Paste it into the environment's setup script; it runs as root before a session.
set -euo pipefail

# Pinned tool versions. Move a pin with a PR that edits this file (R-ci reviews it).
NODE_MAJOR=24
PNPM_VERSION=12.15.0
SPLICE_VERSION=0.2.1
HOIST_VERSION=0.2.0
BERTH_VERSION=0.1.0
# The bosun release this repo pins in .bosun/version.
BOSUN_VERSION=0.2.3

# Rust is not pinned here: the version is the `channel` of the checkout's rust-toolchain.toml.
# Without one the script warns and skips Rust. `bash docs/setup-script.sh --rust-version` prints the version
# and installs nothing.
RUST_VERSION=""
if [ -f rust-toolchain.toml ]; then
  RUST_VERSION="$(sed -nE "s/^[[:space:]]*channel[[:space:]]*=[[:space:]]*[\"']([^\"']*)[\"'].*/\1/p" rust-toolchain.toml | head -n1)"
fi
# The channel reaches rustup as root: only a toolchain name passes, anything else is skipped like a
# missing file (printf %q shows the rejected value safely).
if [ -n "$RUST_VERSION" ] && ! [[ "$RUST_VERSION" =~ ^[A-Za-z0-9][A-Za-z0-9._-]*$ ]]; then
  printf 'setup-script: invalid channel %q in rust-toolchain.toml; skipping the Rust toolchain install and the cargo tools\n' "$RUST_VERSION" >&2
  RUST_VERSION=""
  if [ "${1:-}" = --rust-version ]; then exit 1; fi
elif [ -z "$RUST_VERSION" ]; then
  # Never fail here: a failing setup script blocks every session of the environment.
  echo "setup-script: no rust-toolchain.toml with a channel in $PWD; skipping the Rust toolchain install and the cargo tools, rustup installs the checkout's pinned toolchain on first cargo use" >&2
  if [ "${1:-}" = --rust-version ]; then exit 1; fi
fi
if [ "${1:-}" = --rust-version ]; then echo "$RUST_VERSION"; exit 0; fi

# Node 24 from NodeSource, unless it is already there.
if ! node --version 2>/dev/null | grep -q "^v${NODE_MAJOR}\."; then
  curl -fsSL "https://deb.nodesource.com/setup_${NODE_MAJOR}.x" | bash -
  apt-get install -y nodejs
fi

# pnpm 12 ships a native binary; its install.js puts it in place.
npm install -g "pnpm@${PNPM_VERSION}"
(cd "$(npm root -g)/pnpm" && node install.js) || true

# Rust (the version read above) and the cargo tools the gates run.
if [ -n "$RUST_VERSION" ]; then
  rustup toolchain install "${RUST_VERSION}" --profile minimal --component rustfmt,clippy
  rustup default "${RUST_VERSION}"
  cargo install --locked cargo-nextest cargo-deny
fi

# The keelstackhq tools, pinned.
npm install -g "@keelstackhq/splice@${SPLICE_VERSION}" "@keelstackhq/hoist@${HOIST_VERSION}" \
  "@keelstackhq/berth@${BERTH_VERSION}" "@keelstackhq/bosun@${BOSUN_VERSION}"

bosun --version
