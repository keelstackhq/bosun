#!/usr/bin/env bash
# Setup script for the Claude Code cloud environment of bosun and of repos that use it
# (spec D23). Paste it into the environment's setup script; it runs as root before a session.
set -euo pipefail

# Pinned tool versions. Move a pin with a PR that edits this file (R-ci reviews it).
NODE_MAJOR=24
PNPM_VERSION=12.15.0
RUST_VERSION=1.97
SPLICE_VERSION=0.2.1
HOIST_VERSION=0.2.0
BERTH_VERSION=0.1.0
# The bosun release this repo pins in .bosun/version.
BOSUN_VERSION=0.2.3

# Node 24 from NodeSource, unless it is already there.
if ! node --version 2>/dev/null | grep -q "^v${NODE_MAJOR}\."; then
  curl -fsSL "https://deb.nodesource.com/setup_${NODE_MAJOR}.x" | bash -
  apt-get install -y nodejs
fi

# pnpm 12 ships a native binary; its install.js puts it in place.
npm install -g "pnpm@${PNPM_VERSION}"
(cd "$(npm root -g)/pnpm" && node install.js) || true

# Rust and the cargo tools the gates run.
rustup toolchain install "${RUST_VERSION}" --profile minimal --component rustfmt,clippy
rustup default "${RUST_VERSION}"
cargo install --locked cargo-nextest cargo-deny

# The keelstackhq tools, pinned.
npm install -g "@keelstackhq/splice@${SPLICE_VERSION}" "@keelstackhq/hoist@${HOIST_VERSION}" \
  "@keelstackhq/berth@${BERTH_VERSION}" "@keelstackhq/bosun@${BOSUN_VERSION}"

bosun --version
