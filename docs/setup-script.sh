#!/usr/bin/env bash
# Setup script for the Claude Code cloud environment of bosun and of repos that use it
# (spec D23). Paste it into the environment's setup script; it runs as root before a session.
set -euo pipefail

# Pinned tool versions. Move a pin with a PR that edits this file (R-ci reviews it).
NODE_MAJOR=24
PNPM_VERSION=12.15.0
SPLICE_VERSION=0.4.0
# npm's dist.integrity (sha512) of each tool's launcher and linux binary packages, from
# `npm view @keelstackhq/<package>@<version> dist.integrity`; move them with the version.
SPLICE_SHA512=8/6VRlKzHY3nHFdZnKqGhgyxkj6SLyPeXtqzDgQjTEPlY1m6ZCzGkaXyzuHAj9PYDi1pDOYovlPClyUXRj7dMA==
SPLICE_LINUX_X64_SHA512=0ZtX3mxakLqYUGKFPUZ1xQ8Vd+zkk+YRttlwxZHVSncBBGhe+i4cWCTTbEVjnfNX9qBV0HPfqwQNn2aGV3/suQ==
SPLICE_LINUX_ARM64_SHA512=2UH1s2IWCD9lISISwwFmEhoCgujWmeVdrU1bK5SEPOWq8HPAdUgvSDkoTbHpKd3VrZUjYDTgJ/fsYKu06chNpg==
HOIST_VERSION=0.4.0
HOIST_SHA512=+4m/obpmAmEBq3XM7pDfIkSmk52fmmzpr+OB2Zhq7cJfasNTYJjuooxvLhXoYJ8mqnbkqU48rB9vdKUC/5F2+w==
HOIST_LINUX_X64_SHA512=yfGzHACUGIPVGOyLT2UcXsEAGEM2Khs/rAUgUnhvi34+x8A2s9X5zzLwPJbtoqZ0fs9LFl68KPa+jBidkSnukg==
HOIST_LINUX_ARM64_SHA512=ipN4xVhGm4D9276srARl2EU03xH2uOjlIgRreud2ZB7lI6d0oIjnHcy8cIyo4nqRsM1CSprKxTh2V37l29pf1Q==
BERTH_VERSION=0.1.0
BERTH_SHA512=4V71F1YqcduupUm/yqc3HaK5/VMH6bD1PZa6xskA+84UiSs1UZRnkX7W+zoLPNizoT2CM1bVGmEtk2cjjYg3Eg==
BERTH_LINUX_X64_SHA512=3eaIWInNsevz8vPlkSN+QsPnf82ULEZJz2X4KFOsP3aQR9EanDBDxmm2+J+jEvUBHKxS3xfQMT7HqCO35TUysw==
BERTH_LINUX_ARM64_SHA512=sG6PA8qfOmThoXxL/3P+DSTnr8OL+5MieaTWBv9QfjzcC/ocITUHqi5xptPh2u+71ZY8SqpEcgE6dPmvmi5YXw==
# Claude Code plugins. `keelstack` is bosun's marketplace (.claude-plugin/marketplace.json on this
# branch of keelstackhq/bosun, owner-reviewed); it pins each plugin source by sha. A marketplace ref
# must be a branch or tag (`#<sha>` fails to clone). Move the plugin pins with marketplace.json;
# `bosun doctor` shows drift red.
KEELSTACK_MARKETPLACE_REF=main
SUPERPOWERS_SHA=22f0b463e7a5e579904dd6211a520321035526f7
CAVEMAN_SHA=a9cc1f7b82598852e8c6b8e599dde6ac2491b26e
# The bosun release this repo pins in .bosun/version.
BOSUN_VERSION=0.6.0
BOSUN_SHA512=H3XBylyH1INJgvg5RuSAsYJ4knNtH3mOagiDr+0qDxgfUZOVChV8bJEYCH+uXF+WYJDiBPiAfeTBTZLkyflkJQ==
BOSUN_LINUX_X64_SHA512=CgPRKY4r2bCXtIVwvISyAMBUr6xGGBwXVy42nNHsydP+V7IwFhWYwPLIKlkYpzv9maogVffZX81tyASoJvgWtA==
BOSUN_LINUX_ARM64_SHA512=UIiO210KYO7n1RKZK2MYghckepseecfAnQinK4mR5z1pQTJYSR5IcLfJGv1xqT9rsJrgdtCgjPm4Dk/WboA2Yw==

# Rust is not pinned here: the version is the `channel` of the checkout's rust-toolchain.toml.
# Without one the script warns, skips the pinned toolchain and still installs the cargo tools. `bash docs/setup-script.sh --rust-version` prints the version
# and installs nothing.
RUST_VERSION=""
if [ -f rust-toolchain.toml ]; then
  RUST_VERSION="$(sed -nE "s/^[[:space:]]*channel[[:space:]]*=[[:space:]]*[\"']([^\"']*)[\"'].*/\1/p" rust-toolchain.toml | head -n1)"
fi
# The channel reaches rustup as root: only a toolchain name passes, anything else is skipped like a
# missing file (printf %q shows the rejected value safely).
if [ -n "$RUST_VERSION" ] && ! [[ "$RUST_VERSION" =~ ^[A-Za-z0-9][A-Za-z0-9._-]*$ ]]; then
  printf 'setup-script: invalid channel %q in rust-toolchain.toml; skipping the Rust toolchain install (the cargo tools still install with the default toolchain)\n' "$RUST_VERSION" >&2
  RUST_VERSION=""
  if [ "${1:-}" = --rust-version ]; then exit 1; fi
elif [ -z "$RUST_VERSION" ]; then
  # Never fail here: a failing setup script blocks every session of the environment.
  echo "setup-script: no rust-toolchain.toml with a channel in $PWD; skipping the Rust toolchain install (the cargo tools still install with the default toolchain), rustup installs the checkout's pinned toolchain on first cargo use" >&2
  if [ "${1:-}" = --rust-version ]; then exit 1; fi
fi
if [ "${1:-}" = --rust-version ]; then echo "$RUST_VERSION"; exit 0; fi

# Node 24 from NodeSource, unless it is already there.
if ! node --version 2>/dev/null | grep -q "^v${NODE_MAJOR}\."; then
  curl -fsSL "https://deb.nodesource.com/setup_${NODE_MAJOR}.x" | bash -
  apt-get install -y nodejs
fi

# The image may put an older Node (/opt/node22/bin) before NodeSource's /usr/bin in PATH. Make Node 24 the
# `node` that resolves first, whatever the PATH order: link it into a directory that later shells put
# first (/etc/profile.d), and replace the older `node` that resolves today. Never fails: it warns.
NODE_BIN_DIR="${SETUP_NODE_BIN_DIR:-/usr/bin}"
NODE_LINK_DIR="${SETUP_NODE_LINK_DIR:-/usr/local/lib/bosun-node}"
NODE_PROFILE="${SETUP_NODE_PROFILE:-/etc/profile.d/bosun-node.sh}"
node_major_ok() { "$1" --version 2>/dev/null | grep -q "^v${NODE_MAJOR}\."; }
if ! node_major_ok "$(command -v node || echo node)"; then
  if node_major_ok "$NODE_BIN_DIR/node"; then
    first="$(command -v node || true)"
    {
      mkdir -p "$NODE_LINK_DIR"
      for b in node npm npx; do [ -e "$NODE_BIN_DIR/$b" ] && ln -sf "$NODE_BIN_DIR/$b" "$NODE_LINK_DIR/$b"; done
      printf 'export PATH=%q:"$PATH"\n' "$NODE_LINK_DIR" >"$NODE_PROFILE"
    } 2>/dev/null || echo "setup-script: could not link Node ${NODE_MAJOR} into $NODE_LINK_DIR or write $NODE_PROFILE" >&2
    if [ -n "$first" ] && [ "$first" != "$NODE_BIN_DIR/node" ]; then
      ln -sf "$NODE_BIN_DIR/node" "$first" 2>/dev/null ||
        echo "setup-script: could not replace $first with $NODE_BIN_DIR/node" >&2
    fi
  fi
  if ! node_major_ok "$(command -v node || echo node)"; then
    echo "setup-script: node resolves to $(command -v node || echo none) ($(node --version 2>/dev/null || echo unknown)), not Node ${NODE_MAJOR} at $NODE_BIN_DIR/node" >&2
  fi
fi

# pnpm 12 ships a native binary; its install.js puts it in place.
npm install -g "pnpm@${PNPM_VERSION}"
(cd "$(npm root -g)/pnpm" && node install.js) || true

# Rust (the version read above) and the cargo tools the gates run.
# Without a toolchain file (the checkout does not exist yet when this runs) rustup's stable default
# serves the tools, and rustup installs the checkout's pinned toolchain on first cargo use there.
# A failed step warns and the script goes on: a failing setup script blocks every session.
if [ -n "$RUST_VERSION" ]; then
  { rustup toolchain install "${RUST_VERSION}" --profile minimal --component rustfmt,clippy &&
    rustup default "${RUST_VERSION}"; } ||
    echo "setup-script: installing the Rust toolchain ${RUST_VERSION} failed; continuing" >&2
elif command -v rustup >/dev/null 2>&1 && ! rustup show active-toolchain >/dev/null 2>&1; then
  { rustup toolchain install stable --profile minimal --component rustfmt,clippy &&
    rustup default stable; } ||
    echo "setup-script: installing the stable Rust toolchain failed; continuing" >&2
fi
if command -v cargo >/dev/null 2>&1; then
  cargo install --locked cargo-nextest cargo-deny ||
    echo "setup-script: cargo install of cargo-nextest and cargo-deny failed; continuing without them" >&2
else
  echo "setup-script: no cargo found; skipping cargo-nextest and cargo-deny" >&2
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

# Prewarm the Claude Code plugins (user scope) so the first session does not clone them. Every
# failure, an outage or a pin mismatch, only warns (owner decision): `bosun doctor` shows it red.
# No token: the marketplace and the plugins are public.
if command -v claude >/dev/null 2>&1; then
  claude plugin marketplace add "https://github.com/keelstackhq/bosun.git#$KEELSTACK_MARKETPLACE_REF" >/dev/null ||
    echo "setup-script: adding the keelstack plugin marketplace failed; continuing" >&2
  plugins_file="${CLAUDE_CONFIG_DIR:-${HOME:-/root}/.claude}/plugins/installed_plugins.json"
  # plugin_prewarm <plugin@keelstack> <pinned sha>
  plugin_prewarm() {
    claude plugin install "$1" --scope user >/dev/null ||
      echo "setup-script: claude plugin install $1 failed; continuing" >&2
    got="$(node -e 'try{const p=require(process.argv[1]).plugins[process.argv[2]]||[];process.stdout.write(String((p[0]||{}).gitCommitSha||""))}catch(e){}' "$plugins_file" "$1" 2>/dev/null || true)"
    if [ -z "$got" ]; then
      echo "setup-script: $1 is not installed; cannot check its pin" >&2
    elif [ "$got" != "$2" ]; then
      echo "setup-script: $1 is at $got, not its pinned $2" >&2
    fi
  }
  plugin_prewarm superpowers@keelstack "$SUPERPOWERS_SHA"
  plugin_prewarm caveman@keelstack "$CAVEMAN_SHA"
else
  echo "setup-script: claude is not on PATH; skipping the plugin prewarm" >&2
fi
