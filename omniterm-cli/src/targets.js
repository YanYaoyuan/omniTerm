// Copyright 2026 omniTerm contributors. SPDX-License-Identifier: Apache-2.0
import { readFile, writeFile } from "node:fs/promises";
import { discoverInstances, discoveryDirectory } from "./bridge.js";

export function parseOptions(args, values, booleans = []) {
  const positional = [], options = {};
  for (let index = 0; index < args.length; index++) {
    const item = args[index];
    if (!item.startsWith("--")) { positional.push(item); continue; }
    const split = item.indexOf("=");
    const key = item.slice(2, split < 0 ? undefined : split);
    if (![...values, ...booleans, "help"].includes(key)) throw new Error(`Unknown option: --${key}`);
    if (Object.hasOwn(options, key)) throw new Error(`Duplicate option: --${key}`);
    if (booleans.includes(key) || key === "help") {
      if (split >= 0) throw new Error(`--${key} does not accept a value`);
      options[key] = true;
    } else {
      if (split >= 0) options[key] = item.slice(split + 1);
      else {
        if (args[index + 1] === undefined) throw new Error(`--${key} requires a value`);
        options[key] = args[++index];
      }
    }
  }
  return { positional, options };
}
export function integer(options, key, fallback, max = Number.MAX_SAFE_INTEGER) {
  if (options[key] === undefined) return fallback;
  if (!/^\d+$/.test(options[key])) throw new Error(`--${key} must be a nonnegative integer`);
  const result = Number(options[key]);
  if (!Number.isSafeInteger(result) || result > max) throw new Error(`--${key} is out of range`);
  return result;
}
export async function loadTarget(filename, kind) {
  if (!filename) return undefined;
  const target = JSON.parse(await readFile(filename, "utf8"));
  if (target.version !== 1 || target.kind !== kind || typeof target.instance !== "string") throw new Error(`Invalid ${kind} target binding`);
  if (kind === "terminal" ? typeof target.terminal !== "string" || !Number.isSafeInteger(target.epoch) : typeof target.fileSession !== "string" || typeof target.generation !== "string") throw new Error(`Incomplete ${kind} target binding`);
  return target;
}
export async function saveTarget(filename, target) {
  if (!filename) throw new Error("bind requires --out PATH");
  // Exclusive creation prevents replacing a binding (or an unrelated file) by accident.
  await writeFile(filename, JSON.stringify(target, null, 2) + "\n", { flag: "wx", mode: 0o600 });
}
export async function chooseInstance(options, target, env, platform) {
  if (target && options.instance && options.instance !== target.instance) throw new Error("--instance conflicts with the bound target");
  const instances = await discoverInstances(discoveryDirectory(env, platform));
  const id = target?.instance || options.instance;
  if (!id && instances.length > 1) throw new Error("Several omniTerm instances are running; choose --instance from session instances");
  const selected = id ? instances.find((i) => i.instance === id) : instances.length === 1 ? instances[0] : undefined;
  if (!selected) throw new Error("No matching native omniTerm instance. Restart the updated app from its installed launcher, or check OMNITERM_AUTOMATION_DIR. Bindings from an earlier process must be created again.");
  return selected;
}
export async function poll(result, pending, read, waitMs) {
  const deadline = Date.now() + waitMs;
  while (pending(result) && Date.now() < deadline) {
    await new Promise((resolve) => setTimeout(resolve, Math.min(150, Math.max(1, deadline - Date.now()))));
    result = await read();
  }
  return result;
}
