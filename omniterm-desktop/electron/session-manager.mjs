// Copyright 2026 omniTerm contributors. SPDX-License-Identifier: Apache-2.0

// PTYs, buffers and session limits live in the Rust terminal host. This class keeps what the
// main process owns: command planning and the companion state of remote panels.
function descriptor(session, plan) {
  const { id, title, kind, cwd, status, pid, epoch } = session;
  return { id, title, kind, cwd, status, epoch, ...(pid ? { pid } : {}), ...(plan.remote ? { remote: true } : {}), ...(plan.powershell ? { powershell: true } : {}) };
}

function hostPlan({ executable, args, cwd, env, kind, title, connection }) {
  return { executable, args, cwd, env, kind, title, connection };
}

export class SessionManager {
  constructor({ planner, host, onStart = () => {}, onClose = () => {} }) {
    this.planner = planner;
    this.host = host;
    this.onStart = onStart;
    this.onClose = onClose;
    this.sessions = new Map();
    this.disposed = false;
  }
  async create(request) {
    if (this.disposed) throw new Error("The terminal window is closing.");
    const plan = await this.planner(request);
    if (this.disposed) throw new Error("The terminal window is closing.");
    const session = await this.host.request("session.create", hostPlan(plan));
    this.sessions.set(session.id, { plan, shellNonce: session.shellNonce });
    return descriptor(session, plan);
  }
  require(id) {
    const session = typeof id === "string" ? this.sessions.get(id) : null;
    if (!session) throw new Error("Terminal session was not found.");
    return session;
  }
  async start(id) {
    const { plan } = this.require(id);
    const session = await this.host.request("session.start", { id });
    this.onStart(id, plan);
    return descriptor(session, plan);
  }
  write({ id, data, reply = false } = {}) {
    this.require(id);
    return this.host.request("session.write", { id, data, reply: reply === true });
  }
  resize({ id, cols, rows } = {}) {
    this.require(id);
    return this.host.request("session.resize", { id, cols, rows });
  }
  buffer(id) {
    this.require(id);
    return this.host.request("session.buffer", { id });
  }
  shellNonce(id) { return this.require(id).shellNonce; }
  async close(id) {
    if (typeof id !== "string") throw new Error("Invalid session id.");
    if (!this.sessions.delete(id)) return false;
    this.onClose(id);
    return this.host.request("session.close", { id });
  }
  // A dead host took its PTYs with it, so every tab learns its session ended.
  hostStopped(onExit) {
    for (const id of [...this.sessions.keys()]) onExit({ id, exitCode: 1, signal: null });
  }
  dispose() {
    this.disposed = true;
    for (const id of this.sessions.keys()) this.onClose(id);
    this.sessions.clear();
    this.host.dispose();
  }
}
