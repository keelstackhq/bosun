# Fast cloud setup (design, short)

Owner request 2026-11-10: the cloud setup script takes minutes; make it seconds.

**Why it is slow.** `docs/setup-script.sh` runs `cargo install --locked cargo-nextest cargo-deny`
on every fresh container: both build from source (cargo-deny alone 145 s on 4 cores, measured;
nextest is bigger). Prebuilt downloads from github.com releases (403) and get.nexte.st are blocked
by the default network.

**Approaches.** A: build in the background (setup scripts must not leave daemons). B: prebuilt
binaries (owner must allow hosts). C: repack prebuilt binaries as `@keelstackhq` npm packages
(npm is reachable by default). Embedding cargo-deny as a library: 74 → ~166 crates in a binary
that holds tokens, unstable library API. Owner picked: C for cargo-deny, `cargo test` instead of
nextest.

**Design.**
1. Default rust profile `test` gate: `cargo test --locked` (was `cargo nextest run --locked`).
   A repo may opt back in via `.bosun/profiles/rust.toml`. Channel workflows stop installing
   cargo-nextest. R-weaken by path: owner approves. Note: `cargo test` also runs doctests.
2. Setup script (docs and channel copy): no nextest; Rust steps only when the checkout has a
   `Cargo.toml`; pnpm only when the checkout uses pnpm; cargo-deny from the pinned npm package
   once its pin is set, else `cargo install --locked cargo-deny` as today.
3. `@keelstackhq/cargo-deny` npm packages (launcher + linux-x64 + linux-arm64): a workflow
   downloads the upstream release, checks the upstream sha256, publishes with provenance.
   First publish and the npm trusted publisher are owner steps (Blocked).
4. Follow-up PR after the first publish: set the sha512 pins in both setup scripts.

Left to the builder: exact skip detection, workflow trigger shape (workflow_dispatch with a
version input).
