// Copyright 2026 omniTerm contributors. SPDX-License-Identifier: Apache-2.0
import { spawn } from "node:child_process";
import { accessSync, constants, statSync } from "node:fs";
import path from "node:path";
import { findExecutable, planCommand, resolveCodex, resolveShell } from "../../omniterm-cli/src/cli.js";

export const SESSION_KINDS = Object.freeze(["shell", "agent", "ssh", "sftp", "serial", "tunnel"]);
const PROFILE_FIELDS = new Set(["id", "kind", "name", "host", "username", "port", "keyPath", "device", "baud", "forwardType", "bindPort", "targetHost", "targetPort"]);
const CONTROL = /[\u0000-\u001f\u007f]/;

export function plainObject(value, label) {
  if (!value || typeof value !== "object" || Array.isArray(value)) throw new Error(`${label} must be an object.`);
  return value;
}

function text(value, label, maximum = 1024) {
  if (typeof value !== "string" || value.length > maximum || CONTROL.test(value)) throw new Error(`${label} must be text without control characters (maximum ${maximum} characters).`);
  return value.trim();
}

function integer(value, label, maximum, minimum = 1) {
  if (typeof value === "string" && /^\d+$/.test(value)) value = Number(value);
  if (!Number.isSafeInteger(value) || value < minimum || value > maximum) throw new Error(`${label} must be an integer from ${minimum} to ${maximum}.`);
  return value;
}

function host(value, label, allowUser = false) {
  const result = text(value, label, 255);
  if (!result || result.startsWith("-") || !/^[a-zA-Z0-9_.:@\[\]%-]+$/.test(result) || (!allowUser && result.includes("@")) || result.split("@").length > 2) {
    throw new Error(`${label} must be a hostname, SSH alias, or IP address and cannot start with '-'.`);
  }
  if (result.includes("@") && (!result.split("@")[0] || !result.split("@")[1] || result.split("@")[1].startsWith("-"))) throw new Error(`${label} is not a valid SSH destination.`);
  return result;
}

export function validateProfile(input = {}, { kind = input.kind, requireConnection = false } = {}) {
  plainObject(input, "Profile");
  for (const key of Object.keys(input)) if (!PROFILE_FIELDS.has(key)) throw new Error(`Unsupported profile field: ${key}. Passwords and secrets cannot be saved in profiles.`);
  if (kind !== undefined && !SESSION_KINDS.includes(kind)) throw new Error("Unknown session kind.");
  if (input.kind !== undefined && kind !== input.kind) throw new Error("Profile kind does not match the session kind.");
  const result = kind === undefined ? {} : { kind };
  for (const key of ["id", "name", "keyPath", "device"]) {
    if (input[key] !== undefined && input[key] !== "") result[key] = text(input[key], key, key === "name" ? 120 : 4096);
  }
  if (result.id && !/^[a-zA-Z0-9_-]{1,80}$/.test(result.id)) throw new Error("Invalid profile id.");
  if (input.host !== undefined && input.host !== "") result.host = host(input.host, "Host", true);
  if (input.username !== undefined && input.username !== "") {
    result.username = text(input.username, "Username", 128);
    if (!/^[a-zA-Z0-9_][a-zA-Z0-9_.-]*$/.test(result.username)) throw new Error("Invalid SSH username.");
  }
  if (result.host?.includes("@") && result.username) throw new Error("Enter the username separately or in the host, not both.");
  for (const key of ["port", "bindPort", "targetPort"]) if (input[key] !== undefined && input[key] !== "") result[key] = integer(input[key], key, 65535);
  if (input.baud !== undefined && input.baud !== "") result.baud = integer(input.baud, "Baud rate", 4000000);
  if (input.targetHost !== undefined && input.targetHost !== "") result.targetHost = host(input.targetHost, "Target host");
  if (input.forwardType !== undefined && input.forwardType !== "") {
    result.forwardType = new Map([["L", "L"], ["R", "R"], ["D", "D"], ["local", "L"], ["remote", "R"], ["dynamic", "D"]]).get(input.forwardType);
    if (!result.forwardType) throw new Error("Forward type must be L, R, or D.");
  }
  if (requireConnection) {
    if (["ssh", "sftp", "tunnel"].includes(kind) && !result.host) throw new Error("A host or SSH alias is required.");
    if (kind === "serial" && !result.device) throw new Error("A serial device is required (for example /dev/ttyUSB0 or COM3).");
    if (kind === "serial" && result.device.startsWith("-")) throw new Error("Serial device cannot start with '-'.");
    if (kind === "tunnel") {
      result.forwardType ??= "L";
      if (!result.bindPort) throw new Error("A tunnel listening port is required.");
      if (result.forwardType !== "D" && (!result.targetHost || !result.targetPort)) throw new Error("A target host and port are required for local or remote forwarding.");
    }
  }
  return result;
}

export function externalUrl(value) {
  if (typeof value !== "string" || value.length > 8192) throw new Error("Links must be text of at most 8192 characters.");
  let url;
  try { url = new URL(value); } catch { throw new Error("The link is not a valid URL."); }
  if (!["http:", "https:"].includes(url.protocol)) throw new Error("Only http and https links can be opened.");
  return url.href;
}

function executableFile(filename, platform) {
  try {
    if (!statSync(filename).isFile()) return false;
    accessSync(filename, platform === "win32" ? constants.F_OK : constants.X_OK);
    return true;
  } catch { return false; }
}

export function resolveDesktopAgent(options = {}) {
  const { platform = process.platform, packaged = false, resourcesPath = process.resourcesPath, env = process.env, isExecutable = executableFile } = options;
  const paths = platform === "win32" ? path.win32 : path.posix;
  if (resourcesPath) {
    const executable = paths.join(resourcesPath, "omniterm-agent", "bin", platform === "win32" ? "codex.exe" : "codex");
    if (isExecutable(executable, platform)) return { executable, args: [] };
    if (packaged) throw new Error("The bundled Agent is missing. Reinstall omniTerm with its omniterm-agent resources.");
  } else if (packaged) throw new Error("The packaged Agent resource directory is unavailable.");
  const result = resolveCodex({ ...options, env, platform, isExecutable });
  // Electron's executable is not a normal Node binary. A JS override needs a
  // real Node executable; native packaged/source-built Codex needs none.
  if (result.args.length && /\.[cm]?js$/i.test(result.args[0])) {
    const node = findExecutable("node", { ...options, env, platform, isExecutable });
    if (!node) throw new Error("A JavaScript Codex launcher requires Node on PATH. Use the bundled native Agent or set OMNITERM_CODEX_BIN to a native executable.");
    return { executable: node, args: result.args };
  }
  return result;
}

export function probePyserial(python, env) {
  return new Promise((resolve) => {
    const child = spawn(python.executable, [...python.args, "-c", "import sys; assert sys.version_info.major >= 3; import serial.tools.miniterm"], { env, stdio: "ignore", shell: false, windowsHide: true });
    const timer = setTimeout(() => { child.kill(); resolve(false); }, 5000);
    child.once("error", () => { clearTimeout(timer); resolve(false); });
    child.once("exit", (code) => { clearTimeout(timer); resolve(code === 0); });
  });
}

function destination(profile, kind) {
  let hostname = profile.host;
  const split = hostname.lastIndexOf("@");
  const user = profile.username || (split >= 0 ? hostname.slice(0, split) : "");
  if (split >= 0) hostname = hostname.slice(split + 1);
  if (kind === "sftp" && hostname.includes(":") && !hostname.startsWith("[")) hostname = `[${hostname}]`;
  return user ? `${user}@${hostname}` : hostname;
}

export async function buildCommandPlan(request, options = {}) {
  plainObject(request, "Session request");
  for (const key of Object.keys(request)) if (!["kind", "cwd", "profile"].includes(key)) throw new Error(`Unsupported session field: ${key}.`);
  if (!SESSION_KINDS.includes(request.kind)) throw new Error("Unknown session kind.");
  const { platform = process.platform, env = process.env, defaultCwd = process.cwd(), isDirectory = (filename) => statSync(filename).isDirectory(), serialProbe = probePyserial } = options;
  const paths = platform === "win32" ? path.win32 : path.posix;
  const cwd = request.cwd === undefined || request.cwd === "" ? defaultCwd : text(request.cwd, "Working directory", 4096);
  if (!paths.isAbsolute(cwd)) throw new Error("Working directory must be an absolute path.");
  try { if (!isDirectory(cwd)) throw new Error(); } catch { throw new Error(`Working directory does not exist or is not a directory: ${cwd}`); }
  const profile = validateProfile(request.profile, { kind: request.kind, requireConnection: true });
  let command;
  if (request.kind === "agent") {
    const agent = resolveDesktopAgent(options);
    command = { executable: agent.executable, args: [...agent.args, "--no-daemon", "--no-alt-screen"] };
  } else if (request.kind === "shell") {
    const shell = resolveShell({ ...options, env, platform });
    command = { ...shell, args: platform === "win32" ? shell.args : ["-l"] };
  } else if (request.kind === "serial") {
    command = planCommand("serial", [profile.device, String(profile.baud ?? 115200)], { ...options, env, platform });
    if (!await serialProbe(command.python, env)) throw new Error(`Python 3 with pyserial is required for serial connections. Install pyserial for ${command.python.executable} (python -m pip install pyserial).`);
  } else {
    const args = [];
    if (profile.port) args.push(request.kind === "sftp" ? "-P" : "-p", String(profile.port));
    if (profile.keyPath) args.push("-i", profile.keyPath);
    if (request.kind === "tunnel") {
      let forwarding = `127.0.0.1:${profile.bindPort}`;
      if (profile.forwardType !== "D") {
        const target = profile.targetHost.includes(":") && !profile.targetHost.startsWith("[") ? `[${profile.targetHost}]` : profile.targetHost;
        forwarding += `:${target}:${profile.targetPort}`;
      }
      args.push(`-${profile.forwardType}`, forwarding);
    }
    args.push(destination(profile, request.kind));
    command = planCommand(request.kind, args, { ...options, env, platform });
  }
  return { ...command, kind: request.kind, cwd, title: profile.name || ({ shell: "Terminal", agent: "Agent", ssh: `SSH · ${profile.host}`, sftp: `SFTP · ${profile.host}`, serial: `Serial · ${profile.device}`, tunnel: `Tunnel · ${profile.host}` })[request.kind], env: Object.fromEntries(Object.entries({ ...env, TERM: "xterm-256color", COLORTERM: "truecolor", TERM_PROGRAM: "omniTerm" }).filter(([, value]) => typeof value === "string")) };
}
