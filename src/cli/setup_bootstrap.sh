#!/usr/bin/env bash
# bosun bootstrap: paste it once as a Claude Code cloud environment's setup script; it never
# needs a change. It runs as root before each session, in the directory that holds the checkouts.
# Each checkout (or this directory) runs its own committed docs/setup-script.sh, which pins the
# tool versions and checksums and moves them only by reviewed PRs. It never fails the session.
w() { echo "bosun-bootstrap: $*" >&2; }
case "$(uname -m)" in x86_64 | amd64 | aarch64 | arm64) ;; *) w "untested arch $(uname -m); continuing" ;; esac
found=0
for d in . */; do
  d="${d%/}"
  [ -e "$d/.git" ] && [ -f "$d/docs/setup-script.sh" ] || continue
  found=1
  w "running $d/docs/setup-script.sh"
  (cd "$d" && timeout 1800 bash docs/setup-script.sh </dev/null) ||
    w "$d/docs/setup-script.sh failed (exit $?); continuing"
done
# No repo script and no bosun: the latest bosun, kept only when npm verifies its registry
# signatures and provenance (that needs the host tuf-repo-cdn.sigstore.dev).
p="${BOSUN_BOOTSTRAP_PREFIX:-/usr/local/lib/bosun-latest}"
if [ "$found" = 0 ] && ! command -v bosun >/dev/null 2>&1; then
  if command -v npm >/dev/null 2>&1 && rm -rf "$p" &&
    npm install --prefix "$p" --ignore-scripts --no-audit --no-fund @keelstackhq/bosun@latest >/dev/null 2>&1 &&
    (cd "$p" && npm audit signatures >/dev/null 2>&1) &&
    ln -sf "$p/node_modules/.bin/bosun" "${BOSUN_BOOTSTRAP_BIN:-/usr/local/bin}/bosun"; then
    w "no repo setup script; installed the latest bosun (npm signatures and provenance verified)"
  else
    rm -rf "$p"
    w "no repo setup script and no verified bosun (npm or its signature check failed; allow tuf-repo-cdn.sigstore.dev or run npm i -g @keelstackhq/bosun); continuing"
  fi
fi
exit 0
