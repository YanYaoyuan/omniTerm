// Copyright 2026 omniTerm contributors. SPDX-License-Identifier: Apache-2.0
import { randomUUID } from "node:crypto";
import { readdir, readFile } from "node:fs/promises";
import net from "node:net";
import os from "node:os";
import path from "node:path";

export const sessionHelp = `Control sessions in a running native omniTerm (PR21 or later with bridge).

Usage:
  omniterm session instances
  omniterm session list [--instance ID]
  omniterm session read ID --epoch N [--since SEQ] [--lines N]
  omniterm session setup ID --epoch N --shell bash|zsh
  omniterm session exec ID --epoch N --command 'COMMAND' [--request-id ID]
                         [--timeout-ms N] [--wait-ms N]
  omniterm session result REQUEST_ID [--wait-ms N]
  omniterm session cancel REQUEST_ID
  omniterm session write ID --epoch N --text 'TEXT' [--enter]

Use --command-file PATH or --text-file PATH to read a UTF-8 payload from a file
instead of --command or --text (useful for multiline scripts and shell quoting).

All commands accept --instance ID and return JSON. Choose an instance explicitly
when several are running. Session IDs accept unique prefixes of at least 8 chars.
Setup sends readiness integration and enables SSH auto-submit: only use at an
EMPTY bash 4.4+ / zsh prompt (emacs key bindings). List again to check readiness.
Exec uses completion signals, serializes writes and never blindly retries.
Write is raw input: sent does not mean completed. --enter sends carriage return.
Default wait: 30000 ms; use 0 to return immediately, then poll result.
Exit: 0 success/read/sent, 1 failure, 2 pending/unknown outcome, 3 remote nonzero.
Discovery: OMNITERM_AUTOMATION_DIR, otherwise omniTerm's user-data/automation.
`;

export function discoveryDirectory(env = process.env, platform = process.platform) {
  if (env.OMNITERM_AUTOMATION_DIR) return env.OMNITERM_AUTOMATION_DIR;
  const home = env[platform === "win32" ? "USERPROFILE" : "HOME"] || os.homedir();
  const base = platform === "win32" ? (env.APPDATA || path.join(home, "AppData", "Roaming"))
    : platform === "darwin" ? path.join(home, "Library", "Application Support")
      : env.XDG_CONFIG_HOME && path.isAbsolute(env.XDG_CONFIG_HOME) ? env.XDG_CONFIG_HOME : path.join(home, ".config");
  return path.join(base, "omniTerm", "automation");
}

export function requestBridge(instance, method, params = {}, timeoutMs = 7000) {
  if (instance.protocol !== 1 || !Number.isInteger(instance.port) || instance.port < 1 || instance.port > 65535 || typeof instance.token !== "string") {
    return Promise.reject(new Error("Unsupported or invalid omniTerm discovery file"));
  }
  return new Promise((resolve, reject) => {
    const socket = net.createConnection({ host: "127.0.0.1", port: instance.port });
    let response = "";
    socket.setEncoding("utf8");
    socket.setTimeout(timeoutMs, () => socket.destroy(new Error("omniTerm request timed out; query the same request-id before retrying execution")));
    socket.on("connect", () => socket.write(JSON.stringify({ token: instance.token, method, params }) + "\n"));
    socket.on("error", reject);
    socket.on("data", (chunk) => {
      response += chunk;
      if (response.length > 8 * 1024 * 1024) { socket.destroy(new Error("omniTerm response exceeds 8 MiB")); return; }
      const end = response.indexOf("\n");
      if (end < 0) return;
      socket.end();
      try {
        const value = JSON.parse(response.slice(0, end));
        if (!value.ok) reject(new Error(value.error || "omniTerm request failed"));
        else resolve(value.result);
      } catch (error) { reject(error); }
    });
    socket.on("end", () => { if (!response.includes("\n")) reject(new Error("omniTerm disconnected; query the same request-id before retrying execution")); });
  });
}

export async function discoverInstances(directory) {
  let names;
  try { names = await readdir(directory); }
  catch (error) { if (error.code === "ENOENT") return []; throw error; }
  const candidates = names.filter((name) => name.endsWith(".json"));
  const found = await Promise.all(candidates.map(async (name) => {
    try {
      const instance = JSON.parse(await readFile(path.join(directory, name), "utf8"));
      const ping = await requestBridge(instance, "ping", {}, 500);
      if (ping.pid !== instance.pid) return null;
      return instance;
    } catch { return null; } // Stale files left by a crashed process are harmless.
  }));
  return found.filter(Boolean).sort((a, b) => a.instance.localeCompare(b.instance));
}

function parse(args) {
  const positional = [], options = {};
  for (let index = 0; index < args.length; index++) {
    const item = args[index];
    if (!item.startsWith("--")) { positional.push(item); continue; }
    const key = item.slice(2);
    if (!["instance", "epoch", "since", "lines", "shell", "command", "command-file", "text", "text-file", "enter", "request-id", "timeout-ms", "wait-ms"].includes(key)) throw new Error(`Unknown session option: ${item}`);
    if (Object.hasOwn(options, key)) throw new Error(`Duplicate session option: ${item}`);
    if (key === "enter") options[key] = true;
    else {
      if (args[index + 1] === undefined || args[index + 1].startsWith("--")) throw new Error(`${item} requires a value`);
      options[key] = args[++index];
    }
  }
  return { positional, options };
}
function number(options, key, fallback, max = Number.MAX_SAFE_INTEGER) {
  if (options[key] === undefined) return fallback;
  if (!/^\d+$/.test(options[key])) throw new Error(`--${key} must be a nonnegative integer`);
  const result = Number(options[key]);
  if (!Number.isSafeInteger(result) || result > max) throw new Error(`--${key} is out of range`);
  return result;
}
function resultCode(value) {
  const status = value.operation?.status;
  if (status === "completed") return value.operation.exitCode === 0 ? 0 : value.operation.exitCode == null ? 2 : 3;
  if (["pendingApproval", "approved", "queued", "running", "outcomeUnknown", "takenOver"].includes(status)) return 2;
  return status ? 1 : 0;
}
const pending = (value) => ["pendingApproval", "approved", "queued", "running"].includes(value.operation?.status);

export async function runSession(args, { env = process.env, platform = process.platform, output = (s) => process.stdout.write(s), errorOutput = (s) => process.stderr.write(s) } = {}) {
  let requestId, selected;
  try {
    if (!args.length || args.includes("--help") || args.includes("-h")) { output(sessionHelp); return 0; }
    const { positional, options } = parse(args);
    const [command, id, ...extra] = positional;
    const allowed = {
      instances: [], list: [], read: ["epoch", "since", "lines"], setup: ["epoch", "shell"],
      exec: ["epoch", "command", "command-file", "request-id", "timeout-ms", "wait-ms"],
      result: ["wait-ms"], cancel: [], write: ["epoch", "text", "text-file", "enter"],
    };
    if (!Object.hasOwn(allowed, command)) throw new Error(`Unknown session command: ${command}`);
    for (const key of Object.keys(options)) if (key !== "instance" && !allowed[command].includes(key)) throw new Error(`--${key} is not valid for session ${command}`);
    for (const key of ["command", "text"]) {
      if (options[key + "-file"] !== undefined) {
        if (options[key] !== undefined) throw new Error(`Choose --${key} or --${key}-file, not both`);
        options[key] = await readFile(options[key + "-file"], "utf8");
      }
    }
    if (extra.length || (["instances", "list"].includes(command) ? id !== undefined : !id)) throw new Error(`Invalid arguments for session ${command}; see omniterm session --help`);
    const params = {};
    if (["read", "setup", "exec", "write"].includes(command)) {
      params.terminal = id;
      params.epoch = number(options, "epoch");
      if (params.epoch === undefined) throw new Error("--epoch is required; use session list first");
    }
    if (command === "read") { params.sinceSeq = number(options, "since"); params.maxLines = number(options, "lines", undefined, 2000); }
    if (command === "setup") { if (!["bash", "zsh"].includes(options.shell)) throw new Error("setup requires --shell bash or --shell zsh"); params.shell = options.shell; }
    if (command === "write") { if (options.text === undefined) throw new Error("write requires --text"); params.text = options.text; params.enter = options.enter === true; }
    if (command === "exec") {
      if (!options.command?.trim()) throw new Error("exec requires --command");
      requestId = options["request-id"] || randomUUID();
      const timeoutMs = number(options, "timeout-ms", 120000, 900000);
      if (timeoutMs < 1000) throw new Error("--timeout-ms must be between 1000 and 900000");
      Object.assign(params, { command: options.command, requestId, timeoutMs });
    }
    if (["result", "cancel"].includes(command)) { requestId = id; params.requestId = id; }
    const waitMs = number(options, "wait-ms", 30000, 900000);
    const instances = await discoverInstances(discoveryDirectory(env, platform));
    const publicInstance = ({ instance, pid }) => ({ instance, pid });
    if (command === "instances") { output(JSON.stringify({ instances: instances.map(publicInstance) }) + "\n"); return 0; }
    if (options.instance) selected = instances.find((item) => item.instance === options.instance);
    else if (instances.length === 1) selected = instances[0];
    else if (instances.length > 1) throw new Error("Several omniTerm instances are running; choose --instance from session instances");
    if (!selected) throw new Error("No matching native omniTerm instance. Start the PR21 build with external CLI support, or check OMNITERM_AUTOMATION_DIR");
    let result = await requestBridge(selected, command, params);
    const deadline = Date.now() + waitMs;
    while (["exec", "result"].includes(command) && pending(result) && Date.now() < deadline) {
      await new Promise((resolve) => setTimeout(resolve, Math.min(150, Math.max(1, deadline - Date.now()))));
      result = await requestBridge(selected, "result", { requestId });
    }
    output(JSON.stringify({ instance: selected.instance, ...(requestId ? { requestId } : {}), ...result }) + "\n");
    return resultCode(result);
  } catch (error) {
    errorOutput(JSON.stringify({ error: error.message, ...(selected ? { instance: selected.instance } : {}), ...(requestId ? { requestId, note: "Query session result with this requestId before starting a replacement command." } : {}) }) + "\n");
    return 1;
  }
}
