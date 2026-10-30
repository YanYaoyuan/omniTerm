// Copyright 2026 omniTerm contributors. SPDX-License-Identifier: Apache-2.0
import { randomUUID } from "node:crypto";

const MAX_BUFFER = 2 * 1024 * 1024;
const MAX_SESSIONS = 20;
// PTYs emit many small chunks under heavy output. Coalescing them for a few
// milliseconds keeps IPC traffic per frame bounded without adding visible lag.
const FLUSH_DELAY_MS = 4;
const FLUSH_BYTES = 256 * 1024;

function descriptor(session) {
  return { id: session.id, title: session.plan.title, kind: session.plan.kind, cwd: session.plan.cwd, ...(session.pty ? { pid: session.pty.pid } : {}), status: session.status };
}

export function terminatePty(pty, platform = process.platform, signalProcess = process.kill) {
  // forkpty creates a new session/process group. Signal the group so an Agent's
  // foreground helpers receive hangup too; interactive shells handle their own
  // job-control groups. Windows node-pty closes its ConPTY through kill().
  if (platform !== "win32" && Number.isSafeInteger(pty.pid) && pty.pid > 0) {
    try { signalProcess(-pty.pid, "SIGHUP"); } catch { /* The group may already have exited. */ }
  }
  pty.kill();
}

export class SessionManager {
  constructor({ planner, spawnPty, onData = () => {}, onExit = () => {}, terminate = terminatePty, schedule = (callback) => setTimeout(callback, FLUSH_DELAY_MS), cancel = clearTimeout }) {
    this.planner = planner;
    this.schedule = schedule;
    this.cancel = cancel;
    this.spawnPty = spawnPty;
    this.onData = onData;
    this.onExit = onExit;
    this.terminate = terminate;
    this.sessions = new Map();
    this.disposed = false;
  }
  async create(request) {
    if (this.disposed) throw new Error("The terminal window is closing.");
    if (this.sessions.size >= MAX_SESSIONS) throw new Error("Close a tab before opening another (maximum 20 sessions).");
    const plan = await this.planner(request);
    if (this.disposed) throw new Error("The terminal window is closing.");
    if (this.sessions.size >= MAX_SESSIONS) throw new Error("Close a tab before opening another (maximum 20 sessions).");
    const session = { id: randomUUID(), plan, status: "pending", pty: null, cols: 120, rows: 32, chunks: [], bytes: 0, pending: [], pendingBytes: 0, timer: null, exited: false, exitCode: null, listeners: [] };
    this.sessions.set(session.id, session);
    return descriptor(session);
  }
  require(id) {
    if (typeof id !== "string" || !this.sessions.has(id)) throw new Error("Terminal session was not found.");
    return this.sessions.get(id);
  }
  append(session, data) {
    const chunk = Buffer.from(data, "utf8");
    session.chunks.push(chunk);
    session.bytes += chunk.length;
    while (session.bytes > MAX_BUFFER && session.chunks.length) {
      const first = session.chunks[0];
      const excess = session.bytes - MAX_BUFFER;
      if (first.length <= excess) { session.chunks.shift(); session.bytes -= first.length; }
      else {
        let offset = excess;
        while (offset < first.length && (first[offset] & 0xc0) === 0x80) offset++;
        session.chunks[0] = Buffer.from(first.subarray(offset));
        session.bytes -= offset;
      }
    }
    session.pending.push(data);
    session.pendingBytes += data.length;
    if (session.pendingBytes >= FLUSH_BYTES) this.flush(session);
    else session.timer ??= this.schedule(() => { session.timer = null; this.flush(session); });
  }
  flush(session) {
    if (session.timer !== null) { this.cancel(session.timer); session.timer = null; }
    if (!session.pending.length) return;
    const data = session.pending.length === 1 ? session.pending[0] : session.pending.join("");
    session.pending = [];
    session.pendingBytes = 0;
    if (this.sessions.get(session.id) === session) this.onData({ id: session.id, data });
  }
  start(id) {
    const session = this.require(id);
    if (session.status !== "pending") return descriptor(session);
    session.status = "starting";
    try {
      const pty = this.spawnPty(session.plan.executable, session.plan.args, { name: "xterm-256color", cols: session.cols, rows: session.rows, cwd: session.plan.cwd, env: session.plan.env });
      session.pty = pty;
      session.status = "running";
      session.listeners.push(pty.onData((data) => { if (this.sessions.get(id) === session && !session.exited) this.append(session, data); }));
      session.listeners.push(pty.onExit(({ exitCode, signal }) => {
        if (this.sessions.get(id) !== session || session.exited) return;
        this.flush(session);
        session.exited = true;
        session.status = "exited";
        session.exitCode = exitCode ?? 0;
        this.onExit({ id, exitCode: session.exitCode, signal: signal ?? null });
      }));
      return descriptor(session);
    } catch (error) {
      session.exited = true;
      session.status = "exited";
      session.exitCode = 1;
      this.append(session, `\r\nUnable to start session: ${error.message}\r\n`);
      this.flush(session);
      this.onExit({ id, exitCode: 1, signal: null });
      throw error;
    }
  }
  write({ id, data } = {}) {
    const session = this.require(id);
    if (typeof data !== "string" || Buffer.byteLength(data) > 1024 * 1024) throw new Error("Terminal input must be a string of at most 1 MiB.");
    if (session.status !== "running" || session.exited) return false;
    session.pty.write(data);
    return true;
  }
  resize({ id, cols, rows } = {}) {
    const session = this.require(id);
    if (!Number.isInteger(cols) || cols < 2 || cols > 1000 || !Number.isInteger(rows) || rows < 1 || rows > 500) throw new Error("Terminal size must be 2–1000 columns and 1–500 rows.");
    session.cols = cols;
    session.rows = rows;
    if (session.status === "running" && !session.exited) session.pty.resize(cols, rows);
    return true;
  }
  buffer(id) {
    const session = this.require(id);
    return { data: Buffer.concat(session.chunks, session.bytes).toString("utf8"), exited: session.exited, exitCode: session.exitCode };
  }
  close(id) {
    if (typeof id !== "string") throw new Error("Invalid session id.");
    const session = this.sessions.get(id);
    if (!session) return false;
    if (session.timer !== null) { this.cancel(session.timer); session.timer = null; }
    session.pending = [];
    this.sessions.delete(id);
    for (const listener of session.listeners) listener?.dispose();
    if (session.pty && !session.exited) { try { this.terminate(session.pty); } catch { /* The child may have exited concurrently. */ } }
    return true;
  }
  dispose() {
    this.disposed = true;
    for (const id of [...this.sessions.keys()]) this.close(id);
  }
}
