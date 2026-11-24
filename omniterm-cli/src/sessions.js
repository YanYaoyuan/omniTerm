// Copyright 2026 omniTerm contributors. SPDX-License-Identifier: Apache-2.0
import { randomUUID } from "node:crypto";
import { readFile } from "node:fs/promises";
import { discoverInstances, discoveryDirectory, requestBridge } from "./bridge.js";
export { discoverInstances, discoveryDirectory, requestBridge } from "./bridge.js";
import { parseOptions, integer as number, loadTarget, saveTarget, chooseInstance, poll } from "./targets.js";

export const sessionHelp = `Control sessions in a running native omniTerm (PR21 or later with bridge).

Usage:
  omniterm session instances
  omniterm session list [--instance ID]
  omniterm session bind ID --epoch N --out target.json
  omniterm session info ID --epoch N
  omniterm session search ID --epoch N --query TEXT [--matches N]
  omniterm session wait ID --epoch N --until ready|output [--since SEQ]
  omniterm session read ID --epoch N [--since SEQ] [--lines N]
  omniterm session setup ID --epoch N --shell bash|zsh
  omniterm session exec ID --epoch N --command 'COMMAND' [--request-id ID]
                         [--timeout-ms N] [--wait-ms N]
  omniterm session result REQUEST_ID [--wait-ms N]
  omniterm session cancel REQUEST_ID
  omniterm session write ID --epoch N --text 'TEXT' [--enter]

Use --command-file PATH or --text-file PATH to read a UTF-8 payload from a file
instead of --command or --text (useful for multiline scripts and shell quoting).

Use --target target.json in place of ID and --epoch after binding. Bindings pin
both the instance and connection; reconnecting requires a fresh binding.
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
    if (!args.length || args[0] === "-h") { output(sessionHelp); return 0; }
    const { positional, options } = parseOptions(args, ["instance", "target", "out", "epoch", "since", "lines", "shell", "command", "command-file", "text", "text-file", "request-id", "timeout-ms", "wait-ms", "query", "matches", "until"], ["enter"]);
    if (options.help) { output(sessionHelp); return 0; }
    const [command, id, ...extra] = positional;
    const allowed = {
      instances: [], list: [], info: ["epoch"], bind: ["epoch", "out"],
      read: ["epoch", "since", "lines"], search: ["epoch", "query", "matches"], wait: ["epoch", "since", "lines", "until", "wait-ms"], setup: ["epoch", "shell"],
      exec: ["epoch", "command", "command-file", "request-id", "timeout-ms", "wait-ms"],
      result: ["wait-ms"], cancel: [], write: ["epoch", "text", "text-file", "enter"],
    };
    if (!Object.hasOwn(allowed, command)) throw new Error(`Unknown session command: ${command}`);
    for (const key of Object.keys(options)) if (key !== "instance" && key !== "target" && !allowed[command].includes(key)) throw new Error(`--${key} is not valid for session ${command}`);
    for (const key of ["command", "text"]) {
      if (options[key + "-file"] !== undefined) {
        if (options[key] !== undefined) throw new Error(`Choose --${key} or --${key}-file, not both`);
        options[key] = await readFile(options[key + "-file"], "utf8");
      }
    }
    const target = await loadTarget(options.target, "terminal");
    const scoped = ["info", "bind", "read", "search", "wait", "setup", "exec", "write"].includes(command);
    if (extra.length || (["instances", "list"].includes(command) ? id !== undefined || target !== undefined : !id && !(scoped && target))) throw new Error(`Invalid arguments for session ${command}; see omniterm session --help`);
    if (target && scoped && (id !== undefined || options.epoch !== undefined)) throw new Error("Use --target alone, without a session ID or --epoch");
    const params = {};
    if (scoped) {
      params.terminal = target?.terminal || id;
      params.epoch = target?.epoch ?? number(options, "epoch");
      if (params.epoch === undefined) throw new Error("--epoch is required; use session list first");
    }
    if (command === "read" || command === "wait") { params.sinceSeq = number(options, "since"); params.maxLines = number(options, "lines", undefined, 2000); }
    if (command === "search") { if (!options.query) throw new Error("search requires --query"); params.query = options.query; params.maxMatches = number(options, "matches", 20, 50); }
    if (command === "wait" && options.until && !["ready", "output"].includes(options.until)) throw new Error("--until must be ready or output");
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
    if (command === "instances") {
      const instances = await discoverInstances(discoveryDirectory(env, platform));
      output(JSON.stringify({ instances: instances.map(({ instance, pid }) => ({ instance, pid })) }) + "\n"); return 0;
    }
    selected = await chooseInstance(options, target, env, platform);
    if (command === "bind" && !options.out) throw new Error("bind requires --out PATH");
    let result = await requestBridge(selected, ["bind", "wait"].includes(command) ? "info" : command, params);
    if (command === "bind") {
      const binding = { version: 1, kind: "terminal", instance: selected.instance, terminal: result.ref.sessionId, epoch: result.ref.epoch, label: result.label };
      await saveTarget(options.out, binding);
      result = { binding, targetFile: options.out };
    }
    if (command === "wait") {
      const until = options.until || "ready";
      if (until === "output" && params.sinceSeq === undefined) throw new Error("wait --until output requires --since SEQ");
      const method = until === "ready" ? "info" : "read";
      result = await requestBridge(selected, method, params);
      const waiting = until === "ready" ? (r) => r.interaction?.state !== "ready" : (r) => r.seqEnd <= params.sinceSeq;
      result = await poll(result, waiting, () => requestBridge(selected, method, params), waitMs);
      const ready = !waiting(result);
      output(JSON.stringify({ instance: selected.instance, status: ready ? "ready" : "waiting", until, result }) + "\n");
      return ready ? 0 : 2;
    }
    if (["exec", "result"].includes(command)) result = await poll(result, pending, () => requestBridge(selected, "result", { requestId }), waitMs);
    output(JSON.stringify({ instance: selected.instance, ...(requestId ? { requestId } : {}), ...result }) + "\n");
    return resultCode(result);
  } catch (error) {
    errorOutput(JSON.stringify({ error: error.message, ...(selected ? { instance: selected.instance } : {}), ...(requestId ? { requestId, note: "Query session result with this requestId before starting a replacement command." } : {}) }) + "\n");
    return 1;
  }
}
