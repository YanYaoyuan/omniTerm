// Copyright 2026 omniTerm contributors. SPDX-License-Identifier: Apache-2.0
import assert from "node:assert/strict";
import { spawnSync } from "node:child_process";
import { copyFile, mkdir, mkdtemp, realpath, rm, symlink, writeFile } from "node:fs/promises";
import os from "node:os";
import path from "node:path";
import test from "node:test";
import { fileURLToPath } from "node:url";

const launcher = fileURLToPath(new URL("../skills/omniterm-control/scripts/omniterm.mjs", import.meta.url));
async function fixture(t) {
  const root = await mkdtemp(path.join(os.tmpdir(), "omniterm-skill-"));
  t.after(() => rm(root, { recursive: true, force: true }));
  const copied = path.join(root, "skill", "scripts", "omniterm.mjs");
  await mkdir(path.dirname(copied), { recursive: true });
  await copyFile(launcher, copied);
  const env = { ...process.env, HOME: root, USERPROFILE: root, PATH: "" };
  delete env.OMNITERM_CLI_ENTRY;
  const run = (args, extraEnv = {}) => spawnSync(process.execPath, [copied, ...args], { cwd: root, env: { ...env, ...extraEnv }, encoding: "utf8" });
  const cli = async (name, filesSupport = true) => {
    const packageRoot = path.join(root, name);
    await mkdir(path.join(packageRoot, "bin"), { recursive: true });
    await mkdir(path.join(packageRoot, "src"));
    await writeFile(path.join(packageRoot, "package.json"), JSON.stringify({ name: "@omniterm/cli", type: "module" }));
    await writeFile(path.join(packageRoot, "bin", "omniterm.js"), "// fixture entry\n");
    await writeFile(path.join(packageRoot, "src", "cli.js"), 'export async function main(args) { process.stdout.write(JSON.stringify(args)); return 3; }\n');
    await writeFile(path.join(packageRoot, "src", "sessions.js"), "");
    if (filesSupport) await writeFile(path.join(packageRoot, "src", "files.js"), "");
    return path.join(packageRoot, "bin", "omniterm.js");
  };
  return { root, run, cli };
}

test("bundled skill locates its real CLI and shows help without asset access", async () => {
  const located = spawnSync(process.execPath, [launcher, "--locate"], { encoding: "utf8", env: { ...process.env, OMNITERM_CLI_ENTRY: "" } });
  assert.equal(located.status, 0, located.stderr);
  assert.equal(JSON.parse(located.stdout).entry, await realpath(fileURLToPath(new URL("../bin/omniterm.js", import.meta.url))));
  const help = spawnSync(process.execPath, [launcher, "files", "--help"], { encoding: "utf8", env: { ...process.env, OMNITERM_CLI_ENTRY: "" } });
  assert.equal(help.status, 0, help.stderr);
  assert.match(help.stdout, /omniterm files upload/);
});

test("copied skill honors explicit CLI, preserves literal arguments and exit code", async (t) => {
  const f = await fixture(t);
  const entry = await f.cli("app with spaces");
  const env = { OMNITERM_CLI_ENTRY: entry };
  const located = f.run(["--locate"], env);
  assert.equal(located.status, 0, located.stderr);
  assert.equal(JSON.parse(located.stdout).entry, await realpath(entry));
  const args = ["session", "exec", "--command", "printf '%s' '$HOME; $(unsafe)'\nfalse"];
  const command = f.run(args, env);
  assert.equal(command.status, 3, command.stderr);
  assert.deepEqual(JSON.parse(command.stdout), args);
});

test("invalid override or an old CLI is rejected instead of silently choosing another program", async (t) => {
  const f = await fixture(t);
  const old = await f.cli("old-app", false);
  assert.equal(f.run(["--locate"], { OMNITERM_CLI_ENTRY: old }).status, 1);
  const missing = f.run(["--locate"], { OMNITERM_CLI_ENTRY: path.join(f.root, "gui.exe") });
  assert.equal(missing.status, 1);
  assert.equal(missing.stdout, "");
  assert.match(JSON.parse(missing.stderr).error, /OMNITERM_CLI_ENTRY/);
});

test("standalone skill uses the actual launcher location and rejects ambiguous installations", { skip: process.platform === "win32" }, async (t) => {
  const f = await fixture(t);
  const bin = path.join(f.root, ".local", "bin");
  await mkdir(bin, { recursive: true });
  const first = await f.cli("first-app/omniterm-cli");
  const firstLauncher = path.join(path.dirname(path.dirname(path.dirname(first))), "run.sh");
  await writeFile(firstLauncher, "#!/bin/sh\n");
  await symlink(firstLauncher, path.join(bin, "omniterm-native-pr21"));
  const found = f.run(["--locate"]);
  assert.equal(found.status, 0, found.stderr);
  assert.equal(JSON.parse(found.stdout).entry, await realpath(first));
  const second = await f.cli("second-app/omniterm-cli");
  const secondLauncher = path.join(path.dirname(path.dirname(path.dirname(second))), "run.sh");
  await writeFile(secondLauncher, "#!/bin/sh\n");
  await symlink(secondLauncher, path.join(bin, "omniterm-native-pr22"));
  const ambiguous = f.run(["--locate"]);
  assert.equal(ambiguous.status, 1);
  assert.equal(ambiguous.stdout, "");
  assert.match(JSON.parse(ambiguous.stderr).error, /Multiple/);
});

test("missing installations give actionable discovery errors without running a GUI", async (t) => {
  const f = await fixture(t);
  const result = f.run(["--locate"]);
  assert.equal(result.status, 1);
  assert.equal(result.stdout, "");
  assert.match(JSON.parse(result.stderr).error, /OMNITERM_CLI_ENTRY/);
});


test("CLI discovery canonicalizes an alias without changing which installation it selects", async (t) => {
  const f = await fixture(t);
  const entry = await f.cli("canonical-app");
  const alias = path.join(f.root, "app-alias");
  await symlink(path.dirname(path.dirname(entry)), alias, process.platform === "win32" ? "junction" : "dir");
  const located = f.run(["--locate"], { OMNITERM_CLI_ENTRY: path.join(alias, "bin", "omniterm.js") });
  assert.equal(located.status, 0, located.stderr);
  assert.equal(JSON.parse(located.stdout).entry, await realpath(entry));
});
