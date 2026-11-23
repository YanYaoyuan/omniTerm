// Copyright 2026 omniTerm contributors. SPDX-License-Identifier: Apache-2.0
import { randomUUID } from "node:crypto";
import path from "node:path";
import { requestBridge } from "./bridge.js";
import { parseOptions, integer, loadTarget, saveTarget, chooseInstance, poll } from "./targets.js";

export const filesHelp = `Operate the existing SFTP page in native omniTerm.

Usage:
  omniterm files list [--instance ID]
  omniterm files bind ID --generation G --out files-target.json
  omniterm files info ID --generation G [--offset N] [--limit N]
  omniterm files transfers ID --generation G
  omniterm files cancel-transfer ID --generation G --transfer-id N
  omniterm files ls ID --generation G --path /absolute/path [--offset N] [--limit N]
  omniterm files stat ID --generation G --path /absolute/path
  omniterm files mkdir ID --generation G --path /absolute/path
  omniterm files rename ID --generation G --path /from --to /to [--overwrite]
  omniterm files remove ID --generation G --path /path [--directory]
  omniterm files upload ID --generation G --local PATH --remote /exact/destination
  omniterm files download ID --generation G --remote /source --local PATH
  omniterm files result REQUEST_ID [--wait-ms N]
  omniterm files cancel REQUEST_ID

All commands support --instance; use --target files-target.json instead of ID
and --generation after binding. A reconnect/replacement invalidates the binding.
Remote paths are absolute and destination paths are exact, including the name.
Local paths are resolved on the CLI machine (must share the app's filesystem).
Directories require --recursive. Replacing a destination requires --overwrite.
Remove deletes one file or, with --directory, an empty folder; never recursive.
Operations use --request-id ID (generated if omitted) and --wait-ms N (30000 by
default; 0 returns immediately). Poll result after a timeout; never blindly retry.
ls navigates the page; transfers appear in the page queue and retain progress.
Exit: 0 completed/read, 1 failed/cancelled, 2 pending or outcome unknown.
`;
const pending = (r) => ["queued", "running"].includes(r.operation?.status);
export async function runFiles(args, { env = process.env, platform = process.platform, output = (s) => process.stdout.write(s), errorOutput = (s) => process.stderr.write(s) } = {}) {
  let requestId, selected;
  try {
    if (!args.length || args[0] === "-h") { output(filesHelp); return 0; }
    const { positional, options } = parseOptions(args, ["instance", "target", "generation", "out", "path", "to", "local", "remote", "offset", "limit", "request-id", "wait-ms", "transfer-id"], ["overwrite", "recursive", "directory"]);
    if (options.help) { output(filesHelp); return 0; }
    const [action, id, ...extra] = positional;
    const allowed = { list: [], bind: ["generation", "out"], info: ["generation", "offset", "limit"], transfers: ["generation"], "cancel-transfer": ["generation", "transfer-id"], ls: ["generation", "path", "offset", "limit"], stat: ["generation", "path"], mkdir: ["generation", "path"], rename: ["generation", "path", "to", "overwrite"], remove: ["generation", "path", "directory"], upload: ["generation", "local", "remote", "recursive", "overwrite"], download: ["generation", "local", "remote", "recursive", "overwrite"], result: [], cancel: [] };
    if (!Object.hasOwn(allowed, action)) throw new Error(`Unknown files action: ${action}`);
    const operation = !["list", "info", "bind", "result", "cancel", "transfers", "cancel-transfer"].includes(action);
    for (const key of Object.keys(options)) if (!["instance", "target", "wait-ms", ...(operation ? ["request-id"] : []), ...allowed[action]].includes(key)) throw new Error(`--${key} is not valid for files ${action}`);
    const target = await loadTarget(options.target, "files");
    const scoped = !["list", "result", "cancel"].includes(action);
    if (extra.length || (action === "list" ? id !== undefined || target !== undefined : !id && !(scoped && target))) throw new Error(`Invalid arguments for files ${action}; see files --help`);
    if (scoped && target && (id !== undefined || options.generation !== undefined)) throw new Error("Use --target alone, without a file session ID or --generation");
    const params = {};
    if (scoped) {
      params.fileSession = target?.fileSession || id;
      params.generation = target?.generation || options.generation;
      if (!params.generation) throw new Error("--generation is required; use files list first");
    }
    if (action === "bind" && !options.out) throw new Error("bind requires --out PATH");
    for (const key of ["path", "to", "remote"]) {
      if (options[key] !== undefined) {
        if (!options[key].startsWith("/") || options[key].includes("\0")) throw new Error(`--${key} must be an absolute remote path`);
        params[key] = options[key];
      }
    }
    if (["ls", "stat", "mkdir", "rename", "remove"].includes(action) && !params.path) throw new Error(`${action} requires --path`);
    if (action === "rename" && !params.to) throw new Error("rename requires --to");
    if (["upload", "download"].includes(action)) {
      if (!options.local || !params.remote) throw new Error(`${action} requires --local and --remote`);
      params.local = path.resolve(options.local);
      params.recursive = options.recursive === true;
    }
    if (allowed[action].includes("overwrite")) params.overwrite = options.overwrite === true;
    if (action === "cancel-transfer") { params.transferId = integer(options, "transfer-id"); if (params.transferId === undefined) throw new Error("cancel-transfer requires --transfer-id"); }
    if (action === "remove") params.directory = options.directory === true;
    if (["ls", "info"].includes(action)) { params.offset = integer(options, "offset", 0, 100000); params.limit = integer(options, "limit", 200, 2000); if (params.limit === 0) throw new Error("--limit must be at least 1"); }
    if (operation || ["result", "cancel"].includes(action)) { requestId = operation ? options["request-id"] || randomUUID() : id; params.requestId = requestId; }
    const waitMs = integer(options, "wait-ms", 30000, 900000);
    selected = await chooseInstance(options, target, env, platform);
    let result = await requestBridge(selected, "files." + (action === "bind" ? "info" : action), params);
    if (action === "bind") {
      const binding = { version: 1, kind: "files", instance: selected.instance, fileSession: result.ref.fileSessionId, generation: result.ref.generation, label: result.label };
      await saveTarget(options.out, binding); result = { binding, targetFile: options.out };
    }
    if (operation || action === "result") result = await poll(result, pending, () => requestBridge(selected, "files.result", { requestId }), waitMs);
    output(JSON.stringify({ instance: selected.instance, ...result }) + "\n");
    const status = result.operation?.status;
    return pending(result) || status === "outcomeUnknown" ? 2 : status && status !== "completed" ? 1 : 0;
  } catch (error) {
    errorOutput(JSON.stringify({ error: error.message, ...(selected ? { instance: selected.instance } : {}), ...(requestId ? { requestId, note: "Query files result with this request-id before repeating a file operation." } : {}) }) + "\n");
    return 1;
  }
}
