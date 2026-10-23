"use strict";

const { test } = require("node:test");
const assert = require("node:assert/strict");
const { spawnSync } = require("node:child_process");
const fs = require("node:fs");
const os = require("node:os");
const path = require("node:path");

const LAUNCHER = path.join(__dirname, "..", "bin", "bosun.js");
const launcher = require(LAUNCHER);

test("maps each supported platform to its package", () => {
  assert.equal(launcher.packageFor("linux", "x64"), "@keelstackhq/bosun-linux-x64");
  assert.equal(launcher.packageFor("linux", "arm64"), "@keelstackhq/bosun-linux-arm64");
  assert.equal(launcher.packageFor("darwin", "x64"), "@keelstackhq/bosun-darwin-x64");
  assert.equal(launcher.packageFor("darwin", "arm64"), "@keelstackhq/bosun-darwin-arm64");
  assert.equal(launcher.packageFor("win32", "x64"), "@keelstackhq/bosun-win32-x64");
  assert.equal(launcher.packageFor("win32", "arm64"), undefined);
  assert.equal(launcher.packageFor("freebsd", "x64"), undefined);
});

test("platform packages match the launcher's optionalDependencies", () => {
  const pkg = require("../package.json");
  const names = Object.values(launcher.PLATFORMS).sort();
  assert.deepEqual(Object.keys(pkg.optionalDependencies).sort(), names);
  assert.deepEqual(Object.keys(pkg.bin), ["bosun"]);
});

test("the binary is bin/bosun, with .exe on Windows", () => {
  const resolve = (id) => {
    assert.match(id, /^@keelstackhq\/bosun-[a-z0-9]+-[a-z0-9]+\/package\.json$/);
    return path.join("/pkgs", id);
  };
  assert.equal(
    launcher.binaryPath("linux", "x64", resolve),
    path.join("/pkgs/@keelstackhq/bosun-linux-x64", "bin", "bosun"),
  );
  assert.equal(
    launcher.binaryPath("win32", "x64", resolve),
    path.join("/pkgs/@keelstackhq/bosun-win32-x64", "bin", "bosun.exe"),
  );
});

/** Run the launcher with process.platform/arch faked through a preload module. */
function runAs(platform, arch, args = []) {
  const dir = fs.mkdtempSync(path.join(os.tmpdir(), "bosun-launcher-"));
  const preload = path.join(dir, "fake.js");
  fs.writeFileSync(
    preload,
    `Object.defineProperty(process, "platform", { value: ${JSON.stringify(platform)} });\n` +
      `Object.defineProperty(process, "arch", { value: ${JSON.stringify(arch)} });\n`,
  );
  return spawnSync(process.execPath, ["--require", preload, LAUNCHER, ...args], {
    encoding: "utf8",
  });
}

test("an unknown platform exits 1 and lists the supported ones", () => {
  const r = runAs("aix", "ppc64");
  assert.equal(r.status, 1);
  assert.match(r.stderr, /aix-ppc64/);
  for (const p of ["linux-x64", "linux-arm64", "darwin-x64", "darwin-arm64", "win32-x64"]) {
    assert.ok(r.stderr.includes(p), `stderr lists ${p}: ${r.stderr}`);
  }
});

test("a missing platform package exits 1 and names it", () => {
  const r = runAs("linux", "arm64");
  assert.equal(r.status, 1);
  assert.match(r.stderr, /@keelstackhq\/bosun-linux-arm64/);
});

/** bosun's contract: stdout is exactly one JSON envelope, also when the launcher fails. */
function assertEnvelope(r, command) {
  const env = JSON.parse(r.stdout);
  assert.deepEqual(Object.keys(env).sort(), [
    "command", "data", "error", "exit_code", "next", "ok", "version",
  ]);
  assert.equal(env.version, 1);
  assert.equal(env.command, command);
  assert.equal(env.ok, false);
  assert.equal(env.exit_code, 1);
  assert.deepEqual(env.data, {});
  assert.deepEqual(env.next, []);
  assert.equal(env.error.kind, "error");
  assert.ok(env.error.message.length > 0);
  assert.ok(env.error.fix.length > 0, "error.fix is never empty");
  return env;
}

test("launcher failures print one error envelope on stdout", () => {
  const unknown = assertEnvelope(runAs("aix", "ppc64", ["status", "--json"]), "status");
  assert.match(unknown.error.message, /aix-ppc64/);
  const missing = assertEnvelope(runAs("linux", "arm64", ["verdict", "post"]), "verdict post");
  assert.match(missing.error.fix, /--no-optional|--omit=optional/);
  assertEnvelope(runAs("aix", "ppc64"), "");
});

test("runs the binary with the arguments and passes its exit code through", (t) => {
  if (process.platform === "win32") {
    t.skip("uses a POSIX shell script as the binary");
    return;
  }
  const dir = fs.mkdtempSync(path.join(os.tmpdir(), "bosun-bin-"));
  const pkg = path.join(dir, "node_modules", launcher.packageFor(process.platform, process.arch));
  fs.mkdirSync(path.join(pkg, "bin"), { recursive: true });
  fs.writeFileSync(path.join(pkg, "package.json"), "{}\n");
  const bin = path.join(pkg, "bin", "bosun");
  fs.writeFileSync(bin, '#!/bin/sh\necho "args:$*"\nexit 7\n');
  fs.chmodSync(bin, 0o755);
  const r = spawnSync(
    process.execPath,
    [
      "-e",
      `process.exitCode = require(${JSON.stringify(LAUNCHER)}).run(process.argv.slice(1), ` +
        `(id) => require.resolve(id, { paths: [${JSON.stringify(dir)}] }));`,
      "plan",
      "--json",
    ],
    { encoding: "utf8" },
  );
  assert.equal(r.status, 7, r.stderr);
  assert.equal(r.stdout, "args:plan --json\n");
});

test("a binary without the executable bit is made executable once and run", (t) => {
  if (process.platform === "win32") {
    t.skip("uses a POSIX shell script as the binary");
    return;
  }
  const dir = fs.mkdtempSync(path.join(os.tmpdir(), "bosun-bin-"));
  const pkg = path.join(dir, "node_modules", launcher.packageFor(process.platform, process.arch));
  fs.mkdirSync(path.join(pkg, "bin"), { recursive: true });
  fs.writeFileSync(path.join(pkg, "package.json"), "{}\n");
  const bin = path.join(pkg, "bin", "bosun");
  fs.writeFileSync(bin, '#!/bin/sh\necho "ran:$*"\nexit 3\n');
  fs.chmodSync(bin, 0o644);
  const r = spawnSync(
    process.execPath,
    [
      "-e",
      `process.exitCode = require(${JSON.stringify(LAUNCHER)}).run(process.argv.slice(1), ` +
        `(id) => require.resolve(id, { paths: [${JSON.stringify(dir)}] }));`,
      "check",
    ],
    { encoding: "utf8" },
  );
  assert.equal(r.status, 3, r.stderr);
  assert.equal(r.stdout, "ran:check\n");
  assert.equal(fs.statSync(bin).mode & 0o777, 0o755);
});

test("commandName names the command as bosun's envelope does", () => {
  assert.equal(launcher.commandName(["claim", "12", "--json"]), "claim");
  assert.equal(launcher.commandName(["--json", "verdict", "post", "--from", "-"]), "verdict post");
  assert.equal(launcher.commandName(["wave", "plan", "3"]), "wave plan");
  assert.equal(launcher.commandName(["lesson"]), "lesson");
  assert.equal(launcher.commandName([]), "");
});

test("commandName skips the value of the global -C <DIR> option", () => {
  assert.equal(launcher.commandName(["-C", "/repo", "claim", "1"]), "claim");
  assert.equal(launcher.commandName(["-C", "/repo", "verdict", "post"]), "verdict post");
  assert.equal(launcher.commandName(["-C=/repo", "status"]), "status");
  assert.equal(launcher.commandName(["-C/repo", "wave", "plan", "3"]), "wave plan");
  assert.equal(launcher.commandName(["-v", "-C", "/repo"]), "");
});
