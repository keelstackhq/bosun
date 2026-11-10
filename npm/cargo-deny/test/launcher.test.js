"use strict";

const { test } = require("node:test");
const assert = require("node:assert/strict");
const { spawnSync } = require("node:child_process");
const fs = require("node:fs");
const os = require("node:os");
const path = require("node:path");

const LAUNCHER = path.join(__dirname, "..", "bin", "cargo-deny.js");
const launcher = require(LAUNCHER);

test("maps the two Linux platforms to their packages", () => {
  assert.equal(launcher.packageFor("linux", "x64"), "@keelstackhq/cargo-deny-linux-x64");
  assert.equal(launcher.packageFor("linux", "arm64"), "@keelstackhq/cargo-deny-linux-arm64");
  assert.equal(launcher.packageFor("darwin", "arm64"), undefined);
  assert.equal(launcher.packageFor("win32", "x64"), undefined);
});

test("platform packages match the launcher's optionalDependencies", () => {
  const pkg = require("../package.json");
  assert.deepEqual(Object.keys(pkg.optionalDependencies).sort(), Object.values(launcher.PLATFORMS).sort());
  assert.deepEqual(pkg.bin, { "cargo-deny": "bin/cargo-deny.js" });
});

test("the binary is bin/cargo-deny in the platform package", () => {
  const resolve = (id) => {
    assert.equal(id, "@keelstackhq/cargo-deny-linux-arm64/package.json");
    return path.join("/pkgs", id);
  };
  assert.equal(
    launcher.binaryPath("linux", "arm64", resolve),
    path.join("/pkgs/@keelstackhq/cargo-deny-linux-arm64", "bin", "cargo-deny"),
  );
});

/** Run the launcher with process.platform/arch faked through a preload module. */
function runAs(platform, arch) {
  const dir = fs.mkdtempSync(path.join(os.tmpdir(), "cargo-deny-launcher-"));
  const preload = path.join(dir, "fake.js");
  fs.writeFileSync(
    preload,
    `Object.defineProperty(process, "platform", { value: ${JSON.stringify(platform)} });\n` +
      `Object.defineProperty(process, "arch", { value: ${JSON.stringify(arch)} });\n`,
  );
  return spawnSync(process.execPath, ["--require", preload, LAUNCHER, "check"], { encoding: "utf8" });
}

test("an unsupported platform exits 1 and says how to install instead", () => {
  const r = runAs("darwin", "arm64");
  assert.equal(r.status, 1);
  assert.equal(r.stdout, "");
  assert.match(r.stderr, /darwin-arm64/);
  assert.match(r.stderr, /linux-x64, linux-arm64/);
  assert.match(r.stderr, /cargo install --locked cargo-deny/);
});

test("a missing platform package exits 1 and names it", () => {
  const r = runAs("linux", "arm64");
  assert.equal(r.status, 1);
  assert.match(r.stderr, /@keelstackhq\/cargo-deny-linux-arm64/);
  assert.match(r.stderr, /--omit=optional/);
});

/** A fake platform package for this machine with `bin/cargo-deny` = `script` (mode `mode`). */
function fakeInstall(script, mode) {
  const dir = fs.mkdtempSync(path.join(os.tmpdir(), "cargo-deny-bin-"));
  const pkg = path.join(dir, "node_modules", "@keelstackhq", "cargo-deny-linux-x64");
  fs.mkdirSync(path.join(pkg, "bin"), { recursive: true });
  fs.writeFileSync(path.join(pkg, "package.json"), "{}\n");
  const bin = path.join(pkg, "bin", "cargo-deny");
  fs.writeFileSync(bin, script);
  fs.chmodSync(bin, mode);
  return { dir, bin };
}

/** Run `launcher.run(args)` as linux-x64, resolving packages from `dir`. */
function runWith(dir, args) {
  return spawnSync(
    process.execPath,
    [
      "-e",
      `Object.defineProperty(process, "platform", { value: "linux" });` +
        `Object.defineProperty(process, "arch", { value: "x64" });` +
        `process.exitCode = require(${JSON.stringify(LAUNCHER)}).run(process.argv.slice(1), ` +
        `(id) => require.resolve(id, { paths: [${JSON.stringify(dir)}] }));`,
      "--",
      ...args,
    ],
    { encoding: "utf8" },
  );
}

test("runs the binary with the arguments and passes its exit code through", (t) => {
  if (process.platform === "win32") return t.skip("POSIX shell script as the binary");
  const { dir } = fakeInstall('#!/bin/sh\necho "args:$*"\nexit 7\n', 0o755);
  const r = runWith(dir, ["check", "--hide-inclusion-graph"]);
  assert.equal(r.status, 7, r.stderr);
  assert.equal(r.stdout, "args:check --hide-inclusion-graph\n");
});

test("a binary without the executable bit is made executable once and run", (t) => {
  if (process.platform === "win32") return t.skip("POSIX shell script as the binary");
  const { dir, bin } = fakeInstall('#!/bin/sh\necho "ran:$*"\nexit 0\n', 0o644);
  const r = runWith(dir, ["--version"]);
  assert.equal(r.status, 0, r.stderr);
  assert.equal(r.stdout, "ran:--version\n");
  assert.equal(fs.statSync(bin).mode & 0o777, 0o755);
});
