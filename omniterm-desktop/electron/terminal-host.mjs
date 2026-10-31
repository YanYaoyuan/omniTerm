// Copyright 2026 omniTerm contributors. SPDX-License-Identifier: Apache-2.0
import { spawn } from "node:child_process";
import { accessSync, constants } from "node:fs";
import path from "node:path";

export const PROTOCOL_VERSION = 1;
const KIND_JSON = 1;
const KIND_DATA = 2;
const MAX_FRAME = 64 * 1024 * 1024;

const isExecutable = (filename) => { try { accessSync(filename, constants.X_OK); return true; } catch { return false; } };

export function resolveTerminalHost({ packaged = false, resourcesPath, desktopRoot, platform = process.platform, env = process.env, exists = isExecutable }) {
  const name = platform === "win32" ? "omniterm-terminal-host.exe" : "omniterm-terminal-host";
  const paths = platform === "win32" ? path.win32 : path.posix;
  const packagedHost = resourcesPath ? paths.join(resourcesPath, "terminal-host", name) : null;
  if (packaged) {
    if (packagedHost && exists(packagedHost)) return packagedHost;
    throw new Error("The bundled terminal host is missing. Reinstall omniTerm.");
  }
  const candidates = [env.OMNITERM_TERMINAL_HOST, paths.join(desktopRoot, "native/terminal-host/target/release", name), paths.join(desktopRoot, "native/terminal-host/target/debug", name)].filter(Boolean);
  const found = candidates.find((candidate) => exists(candidate));
  if (!found) throw new Error("The terminal host is not built. Run npm run build:native in omniterm-desktop.");
  return found;
}

// Stdout is a byte stream, so frames can arrive split or merged in any chunk.
export class FrameDecoder {
  constructor() { this.chunks = []; this.size = 0; }
  push(chunk) {
    this.chunks.push(chunk);
    this.size += chunk.length;
    const frames = [];
    if (this.size < 4) return frames;
    let buffer = this.chunks.length === 1 ? this.chunks[0] : Buffer.concat(this.chunks, this.size);
    let offset = 0;
    while (buffer.length - offset >= 4) {
      const length = buffer.readUInt32LE(offset);
      if (length < 1 || length > MAX_FRAME) throw new Error("The terminal host sent a corrupt frame.");
      if (buffer.length - offset - 4 < length) break;
      frames.push(decodeFrame(buffer.subarray(offset + 4, offset + 4 + length)));
      offset += 4 + length;
    }
    const rest = offset === buffer.length ? null : buffer.subarray(offset);
    this.chunks = rest ? [rest] : [];
    this.size = rest ? rest.length : 0;
    return frames;
  }
}

function decodeFrame(frame) {
  if (frame[0] === KIND_JSON) return { json: JSON.parse(frame.toString("utf8", 1)) };
  if (frame[0] === KIND_DATA) {
    const idLength = frame[1];
    const dataStart = 2 + idLength + 8;
    return { data: { id: frame.toString("utf8", 2, 2 + idLength), seq: Number(frame.readBigUInt64LE(2 + idLength)), data: frame.toString("utf8", dataStart) } };
  }
  throw new Error("The terminal host sent an unknown frame.");
}

export class TerminalHost {
  constructor({ executable, spawnProcess = spawn, onData = () => {}, onEvent = () => {}, onExit = () => {} }) {
    this.executable = executable;
    this.spawnProcess = spawnProcess;
    this.onData = onData;
    this.onEvent = onEvent;
    this.onExit = onExit;
    this.pending = new Map();
    this.nextId = 1;
    this.child = null;
    this.disposed = false;
  }
  ensureStarted() {
    if (this.child) return;
    if (this.disposed) throw new Error("The terminal window is closing.");
    const child = this.spawnProcess(this.executable, [], { stdio: ["pipe", "pipe", "inherit"], windowsHide: true });
    const decoder = new FrameDecoder();
    this.child = child;
    child.stdout.on("data", (chunk) => {
      let frames;
      try { frames = decoder.push(chunk); }
      catch (error) { child.kill(); this.failAll(error); return; }
      for (const frame of frames) this.dispatch(frame);
    });
    child.stdin.on("error", () => { /* Reported through the exit event. */ });
    const ended = (message) => {
      if (this.child !== child) return;
      this.child = null;
      this.failAll(new Error(message));
      if (!this.disposed) this.onExit();
    };
    child.once("error", (error) => ended(`The terminal host could not run: ${error.message}`));
    child.once("exit", () => ended("The terminal host stopped."));
  }
  dispatch({ json, data }) {
    if (data) { this.onData(data); return; }
    if (json.event) {
      if (json.event === "host.ready" && json.params?.protocolVersion !== PROTOCOL_VERSION) {
        this.failAll(new Error("The terminal host does not match this version of omniTerm."));
        this.child?.kill();
        return;
      }
      this.onEvent(json.event, json.params);
      return;
    }
    const request = this.pending.get(json.id);
    if (!request) return;
    this.pending.delete(json.id);
    if (json.error) request.reject(new Error(json.error.message));
    else request.resolve(json.result);
  }
  failAll(error) {
    for (const { reject } of this.pending.values()) reject(error);
    this.pending.clear();
  }
  request(method, params = {}) {
    return new Promise((resolve, reject) => {
      this.ensureStarted();
      const id = this.nextId++;
      this.pending.set(id, { resolve, reject });
      this.child.stdin.write(`${JSON.stringify({ id, method, params })}\n`);
    });
  }
  // Closing stdin makes the host end every PTY, and it does the same if this process dies.
  dispose() {
    this.disposed = true;
    this.child?.stdin.end();
    this.child = null;
    this.failAll(new Error("The terminal window is closing."));
  }
}
