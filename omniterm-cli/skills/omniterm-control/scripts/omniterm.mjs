#!/usr/bin/env node
// Copyright 2026 omniTerm contributors. SPDX-License-Identifier: Apache-2.0
// Works both inside the CLI bundle and when this entire skill is copied elsewhere.
import { readdir, readFile, realpath, stat } from "node:fs/promises";
import os from "node:os";
import path from "node:path";
import { fileURLToPath, pathToFileURL } from "node:url";

async function entryAt(candidate) {
  try {
    const entry = await realpath(candidate);
    const root = path.dirname(path.dirname(entry));
    const metadata = JSON.parse(await readFile(path.join(root, "package.json"), "utf8"));
    if (metadata.name !== "@omniterm/cli" || path.basename(entry) !== "omniterm.js" || path.basename(path.dirname(entry)) !== "bin") return undefined;
    for (const name of ["src/cli.js", "src/sessions.js", "src/files.js"]) {
      if (!(await stat(path.join(root, name))).isFile()) return undefined;
    }
    return entry;
  } catch { return undefined; }
}
async function choose(candidates) {
  const found = new Set();
  for (const candidate of candidates) {
    const entry = await entryAt(candidate);
    if (entry) found.add(entry);
  }
  if (found.size > 1) throw new Error("Multiple omniTerm CLI installations found; set OMNITERM_CLI_ENTRY to the intended bin/omniterm.js: " + [...found].join(", "));
  return [...found][0];
}
async function names(directory) {
  try { return await readdir(directory); } catch (error) {
    if (["ENOENT", "ENOTDIR"].includes(error.code)) return [];
    throw error;
  }
}
async function locate() {
  if (process.env.OMNITERM_CLI_ENTRY) {
    const explicit = await entryAt(process.env.OMNITERM_CLI_ENTRY);
    if (!explicit) throw new Error("OMNITERM_CLI_ENTRY must point to a current @omniterm/cli bin/omniterm.js with terminal and files support");
    return explicit;
  }
  const own = fileURLToPath(new URL("../../../bin/omniterm.js", import.meta.url));
  const bundled = await entryAt(own);
  if (bundled) return bundled;

  const home = process.env[process.platform === "win32" ? "USERPROFILE" : "HOME"] || os.homedir();
  const launchers = [];
  const localBin = path.join(home, ".local", "bin");
  for (const name of await names(localBin)) {
    if (!/^omniterm-native(?:-pr\d+)?$/.test(name)) continue;
    try {
      const launcher = await realpath(path.join(localBin, name));
      launchers.push(path.join(path.dirname(launcher), "omniterm-cli", "bin", "omniterm.js"));
    } catch { /* A broken shortcut is not an installed CLI. */ }
  }
  const installed = await choose(launchers);
  if (installed) return installed;

  const onPath = await choose((process.env.PATH || "").split(path.delimiter).filter(Boolean).map((directory) => path.join(directory, "omniterm")));
  if (onPath) return onPath;
  for (let directory = process.cwd(); ; directory = path.dirname(directory)) {
    const workspace = await choose([
      path.join(directory, "dist", "local-native", "omniterm-cli", "bin", "omniterm.js"),
      path.join(directory, "omniterm-cli", "bin", "omniterm.js"),
    ]);
    if (workspace) return workspace;
    if (path.dirname(directory) === directory) break;
  }
  const parent = path.join(home, ".local", "share", "omniterm-native");
  const versions = await names(parent);
  const fallback = await choose(versions.map((version) => path.join(parent, version, "omniterm-cli", "bin", "omniterm.js")));
  if (fallback) return fallback;
  throw new Error("omniTerm CLI was not found. Install/unpack the native CLI bundle and set OMNITERM_CLI_ENTRY to its bin/omniterm.js. Node.js 22+ is required.");
}
try {
  const entry = await locate();
  const args = process.argv.slice(2);
  if (args[0] === "--locate") {
    if (args.length !== 1) throw new Error("Use --locate alone; other arguments are passed to the CLI unchanged");
    process.stdout.write(JSON.stringify({ entry }) + "\n");
  } else {
    const module = await import(pathToFileURL(path.join(path.dirname(entry), "../src/cli.js")).href);
    if (typeof module.main !== "function") throw new Error("The selected CLI does not export main(argv); update the CLI bundle");
    process.exitCode = await module.main(args);
  }
} catch (error) {
  process.stderr.write(JSON.stringify({ error: error.message }) + "\n");
  process.exitCode = 1;
}
