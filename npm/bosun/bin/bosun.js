#!/usr/bin/env node
"use strict";

// npm launcher for bosun: runs the binary from the platform package npm installed through
// optionalDependencies, with inherited stdio and the binary's exit code. When it cannot start
// the binary it keeps bosun's contract: one JSON envelope on stdout, the message on stderr.

const { spawnSync } = require("node:child_process");
const fs = require("node:fs");
const path = require("node:path");

/** `process.platform-process.arch` -> platform package (one per release target). */
const PLATFORMS = {
  "linux-x64": "@keelstackhq/bosun-linux-x64",
  "linux-arm64": "@keelstackhq/bosun-linux-arm64",
  "darwin-x64": "@keelstackhq/bosun-darwin-x64",
  "darwin-arm64": "@keelstackhq/bosun-darwin-arm64",
  "win32-x64": "@keelstackhq/bosun-win32-x64",
};

function packageFor(platform, arch) {
  return PLATFORMS[`${platform}-${arch}`];
}

/** Path of `bin/bosun[.exe]` inside the platform package; throws when it is not installed. */
function binaryPath(platform, arch, resolve = require.resolve) {
  const pkg = packageFor(platform, arch);
  const dir = path.dirname(resolve(`${pkg}/package.json`));
  return path.join(dir, "bin", platform === "win32" ? "bosun.exe" : "bosun");
}

/** Commands with a subcommand; their envelope names both words ("verdict post"). */
const GROUPS = new Set(["wave", "verdict", "lesson", "hook", "budget"]);

/** The envelope's `command` for `args`, as bosun names it; "" when there is none. */
function commandName(args) {
  const words = [];
  for (let i = 0; i < args.length; i++) {
    // `-C <DIR>` is bosun's one global option that takes a value (also `-C=DIR`, `-CDIR`).
    if (args[i] === "-C") i++;
    else if (!args[i].startsWith("-")) words.push(args[i]);
  }
  if (words.length === 0) return "";
  return GROUPS.has(words[0]) && words.length > 1 ? `${words[0]} ${words[1]}` : words[0];
}

/** Report a launcher failure: the message on stderr, one error envelope on stdout; returns 1. */
function fail(args, message, fix) {
  process.stderr.write(`bosun: ${message}\n`);
  const envelope = {
    version: 1,
    command: commandName(args),
    ok: false,
    exit_code: 1,
    data: {},
    error: { kind: "error", message, fix },
    next: [],
  };
  process.stdout.write(`${JSON.stringify(envelope)}\n`);
  return 1;
}

/** Run bosun with `args`; returns the exit code. */
function run(args, resolve = require.resolve) {
  const { platform, arch } = process;
  const key = `${platform}-${arch}`;
  const pkg = packageFor(platform, arch);
  if (!pkg) {
    return fail(
      args,
      `no binary for ${key}; supported platforms: ${Object.keys(PLATFORMS).join(", ")}`,
      "use a supported platform, or build from source: cargo install keel-bosun",
    );
  }
  let bin;
  try {
    bin = binaryPath(platform, arch, resolve);
  } catch {
    return fail(
      args,
      `platform package ${pkg} is not installed`,
      "reinstall @keelstackhq/bosun without --no-optional (or --omit=optional)",
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
    return fail(
      args,
      `cannot run ${bin}: ${result.error.message}`,
      "reinstall @keelstackhq/bosun, or build from source: cargo install keel-bosun",
    );
  }
  if (result.signal) {
    process.kill(process.pid, result.signal);
    return 1;
  }
  return result.status ?? 1;
}

module.exports = { PLATFORMS, packageFor, binaryPath, commandName, run };

if (require.main === module) {
  process.exitCode = run(process.argv.slice(2));
}
