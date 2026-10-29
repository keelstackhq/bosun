#!/usr/bin/env bash
# Setup script for the Claude Code cloud environment of bosun and of repos that use it
# (spec D23). Paste it into the environment's setup script; it runs as root before a session.
set -euo pipefail

# Pinned tool versions. Move a pin with a PR that edits this file (R-ci reviews it).
NODE_MAJOR=24
PNPM_VERSION=12.15.0
SPLICE_VERSION=0.3.1
# npm's dist.integrity (sha512) of each tool's launcher and linux binary packages, from
# `npm view @keelstackhq/<package>@<version> dist.integrity`; move them with the version.
SPLICE_SHA512=FyJMvUQ4SxjaLtECPp1YmRpMMzYwRaGsdCJ/den/PRZBkkecMyWRLq3v7UECIpJfWuIGWlDmWz48l6X9cNsxeA==
SPLICE_LINUX_X64_SHA512=OtpywXIof4d/eP7dhvt8q3hh5k65qCpE7YtTh9EmA9RqtPtFxksBaxWT3FjWHkqJ7RQm7JCKme2xIBVnBpAhHA==
SPLICE_LINUX_ARM64_SHA512=gJ4y/6sOgGooEkf1flpUUIt9u77T4oe7CBKzCEWXSeGwHyRH7jE3/v/lHFKtaGRF5OhW7sq+wbN3z7uCBQIkxQ==
HOIST_VERSION=0.3.0
HOIST_SHA512=0OO9wf3zQgSmZzGLUkLvjrdhQsUq1zGtyJ4CDi/EprxDmOFxvyxJfPpEDIez0xvwN9UNqhjwDw9x77GrFXeMVQ==
HOIST_LINUX_X64_SHA512=dfsuZovno5ZMtepOLDhYiLQLu9K0hTdAgVVsO4VhwKw6fWnH4PGE3Ks/sqnShT+ajSzN25pETEVV9euIZvxcag==
HOIST_LINUX_ARM64_SHA512=dFY+6R78093CTKpipt3I7c037oz2SDZ8cUgJjH00OXkbrdzTWgHqh7wytOs9/JF56aHW38K1ATS0L0jc21AmVg==
BERTH_VERSION=0.1.0
BERTH_SHA512=4V71F1YqcduupUm/yqc3HaK5/VMH6bD1PZa6xskA+84UiSs1UZRnkX7W+zoLPNizoT2CM1bVGmEtk2cjjYg3Eg==
BERTH_LINUX_X64_SHA512=3eaIWInNsevz8vPlkSN+QsPnf82ULEZJz2X4KFOsP3aQR9EanDBDxmm2+J+jEvUBHKxS3xfQMT7HqCO35TUysw==
BERTH_LINUX_ARM64_SHA512=sG6PA8qfOmThoXxL/3P+DSTnr8OL+5MieaTWBv9QfjzcC/ocITUHqi5xptPh2u+71ZY8SqpEcgE6dPmvmi5YXw==
# The bosun release this repo pins in .bosun/version.
BOSUN_VERSION=0.3.0
BOSUN_SHA512=9qaNf4PstfjZ92uMdChV9gY62Rb+wlCANRrcoBsN96MfGTf5g+gA+NdVXlngFlOVDynJZd7/rjW1hHyFoE1eYw==
BOSUN_LINUX_X64_SHA512=a+LmXD+fvpcXosaUm0bOnjCqZ4cy0DziAQRtI/havMH/nXNbpzXZPdn5c4yg59f/ff/qKA+6dwSItiUftJA/lg==
BOSUN_LINUX_ARM64_SHA512=/aQa03VNgR8k1mYoscze8nW62MNuMFJW28ZXwR6ubkpEJRBR5MeUuEU5dNPVsWH/D3AvHIgdiU4GA5dI9Rcqvg==

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

# The keelstackhq tools, pinned, install only from npm tarballs that match the pinned sha512:
# each launcher and this machine's binary package, side by side. A mismatch or a failed install
# skips that tool with a warning (a failing setup script blocks every session).
tool_pack() {
  f="$(cd "$2" && npm pack --silent "$1")"
  got="$(node -e 'process.stdout.write(require("crypto").createHash("sha512").update(require("fs").readFileSync(process.argv[1])).digest("base64"))' "$2/$f")"
  if [ "$got" != "$3" ]; then echo "setup-script: $1 does not match its pinned sha512; skipping it" >&2; return 1; fi
  echo "$2/$f"
}
# install_tool <name> <version> <launcher sha512> <linux-x64 sha512> <linux-arm64 sha512>
install_tool() {
  case "$(uname -m)" in aarch64 | arm64) a=arm64 s="$5" ;; *) a=x64 s="$4" ;; esac
  t="$(mktemp -d)"
  if l="$(tool_pack "@keelstackhq/$1@$2" "$t" "$3")" && b="$(tool_pack "@keelstackhq/$1-linux-$a@$2" "$t" "$s")"; then
    # npm still reads the optional packages' metadata: a registry error must not stop the script.
    npm install -g --ignore-scripts --omit=optional "$l" "$b" ||
      echo "setup-script: $1 install failed; skipping $1" >&2
  fi
}
install_tool splice "$SPLICE_VERSION" "$SPLICE_SHA512" "$SPLICE_LINUX_X64_SHA512" "$SPLICE_LINUX_ARM64_SHA512"
install_tool hoist "$HOIST_VERSION" "$HOIST_SHA512" "$HOIST_LINUX_X64_SHA512" "$HOIST_LINUX_ARM64_SHA512"
install_tool berth "$BERTH_VERSION" "$BERTH_SHA512" "$BERTH_LINUX_X64_SHA512" "$BERTH_LINUX_ARM64_SHA512"
install_tool bosun "$BOSUN_VERSION" "$BOSUN_SHA512" "$BOSUN_LINUX_X64_SHA512" "$BOSUN_LINUX_ARM64_SHA512"

bosun --version || echo "setup-script: bosun is not installed" >&2
