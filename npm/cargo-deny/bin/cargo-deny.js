#!/usr/bin/env node
"use strict";

// npm launcher for upstream cargo-deny (EmbarkStudios/cargo-deny, MIT OR Apache-2.0): runs the
// prebuilt musl binary from the platform package npm installed through optionalDependencies,
// with inherited stdio and the binary's exit code. Republished by
// .github/workflows/cargo-deny-npm.yml in keelstackhq/bosun; the binary is upstream's release
// asset, unchanged.

const { spawnSync } = require("node:child_process");
const fs = require("node:fs");
const path = require("node:path");

/** `process.platform-process.arch` -> platform package (upstream's static musl builds). */
const PLATFORMS = {
  "linux-x64": "@keelstackhq/cargo-deny-linux-x64",
  "linux-arm64": "@keelstackhq/cargo-deny-linux-arm64",
};

const FALLBACK = "install it with: cargo install --locked cargo-deny";

function packageFor(platform, arch) {
  return PLATFORMS[`${platform}-${arch}`];
}

/** Path of `bin/cargo-deny` inside the platform package; throws when it is not installed. */
function binaryPath(platform, arch, resolve = require.resolve) {
  const dir = path.dirname(resolve(`${packageFor(platform, arch)}/package.json`));
  return path.join(dir, "bin", "cargo-deny");
}

function fail(message) {
  process.stderr.write(`cargo-deny (npm launcher): ${message}\n`);
  return 1;
}

/** Run cargo-deny with `args`; returns the exit code. */
function run(args, resolve = require.resolve) {
  const { platform, arch } = process;
  const pkg = packageFor(platform, arch);
  if (!pkg) {
    const supported = Object.keys(PLATFORMS).join(", ");
    return fail(`no binary for ${platform}-${arch}; supported platforms: ${supported}; ${FALLBACK}`);
  }
  let bin;
  try {
    bin = binaryPath(platform, arch, resolve);
  } catch {
    return fail(
      `platform package ${pkg} is not installed; reinstall @keelstackhq/cargo-deny ` +
        "without --no-optional (or --omit=optional)",
    );
  }
  let result = spawnSync(bin, args, { stdio: "inherit" });
  if (result.error && result.error.code === "EACCES") {
    // Some installers drop the executable bit; restore it once and retry.
    try {
      fs.chmodSync(bin, 0o755);
      result = spawnSync(bin, args, { stdio: "inherit" });
    } catch {
      // Keep the first error.
    }
  }
  if (result.error) {
    return fail(`cannot run ${bin}: ${result.error.message}; reinstall @keelstackhq/cargo-deny, or ${FALLBACK}`);
  }
  if (result.signal) {
    process.kill(process.pid, result.signal);
    return 1;
  }
  return result.status ?? 1;
}

module.exports = { PLATFORMS, packageFor, binaryPath, run };

if (require.main === module) {
  process.exitCode = run(process.argv.slice(2));
}
