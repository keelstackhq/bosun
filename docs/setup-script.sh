#!/usr/bin/env bash
# Setup script for the Claude Code cloud environment of bosun and of repos that use it
# (spec D23). Paste it into the environment's setup script; it runs as root before a session.
set -euo pipefail

# Pinned tool versions. Move a pin with a PR that edits this file (R-ci reviews it).
NODE_MAJOR=24
PNPM_VERSION=12.15.0
SPLICE_VERSION=0.3.0
# npm's dist.integrity (sha512) of splice 0.3.0's launcher and linux binary packages, from
# `npm view @keelstackhq/<package>@0.3.0 dist.integrity`; move them with SPLICE_VERSION.
SPLICE_SHA512=NmTKFPnwnLN0fNFdt7yUAikv+XXgFd3DV/kJzBTxWkXdwJn7geSl5tDmr71B3pEofjQ3V7SxMfejZcff1eAlsA==
SPLICE_LINUX_X64_SHA512=lx56ekCHokRBLZr8iuUpRh4fG8YO99m8VXJUHiHmjZ3vx9VpqX1QtLRbplpL1lHbN36ovZqTlAgAIZJsoJ5sUQ==
SPLICE_LINUX_ARM64_SHA512=NS1EGfcTkR2RYfnb0LH0+/zpXtbaJMKUpw+CgLWDcpnv+2K/ClLAjiVX7iUenHhaA6H4vOjC4aWSf52jL4xc4g==
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
if [ -z "$RUST_VERSION" ]; then
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

# The keelstackhq tools, pinned. splice (it writes the repo's files) installs only from tarballs
# that match the pinned sha512: the launcher and this machine's binary package, side by side. A
# mismatch skips splice with a warning (a failing setup script blocks every session).
splice_pack() {
  f="$(cd "$2" && npm pack --silent "$1@${SPLICE_VERSION}")"
  got="$(node -e 'process.stdout.write(require("crypto").createHash("sha512").update(require("fs").readFileSync(process.argv[1])).digest("base64"))' "$2/$f")"
  if [ "$got" != "$3" ]; then echo "setup-script: $1@${SPLICE_VERSION} does not match its pinned sha512; skipping splice" >&2; return 1; fi
  echo "$2/$f"
}
case "$(uname -m)" in aarch64 | arm64) arch=arm64 sum="$SPLICE_LINUX_ARM64_SHA512" ;; *) arch=x64 sum="$SPLICE_LINUX_X64_SHA512" ;; esac
t="$(mktemp -d)"
if launcher="$(splice_pack @keelstackhq/splice "$t" "$SPLICE_SHA512")" &&
  binary="$(splice_pack "@keelstackhq/splice-linux-$arch" "$t" "$sum")"; then
  npm install -g --ignore-scripts --omit=optional "$launcher" "$binary"
fi
npm install -g "@keelstackhq/hoist@${HOIST_VERSION}" "@keelstackhq/berth@${BERTH_VERSION}" \
  "@keelstackhq/bosun@${BOSUN_VERSION}"

bosun --version
