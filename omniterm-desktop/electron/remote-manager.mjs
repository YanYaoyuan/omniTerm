// Copyright 2026 omniTerm contributors. SPDX-License-Identifier: Apache-2.0
// The companion connection behind an SSH tab: an SFTP channel for the file panel
// and a metrics channel for the host bar. Both share the terminal's authenticated
// OpenSSH connection through its control socket, so they never ask for credentials
// and passwords never reach this process. Windows OpenSSH has no control sockets;
// there the channels open their own non-interactive connection instead.
import { randomUUID } from "node:crypto";
import { spawn, spawnSync } from "node:child_process";
import { chmodSync, existsSync, mkdtempSync, rmSync } from "node:fs";
import { lstat, mkdir, mkdtemp, readdir, rename, rm, stat } from "node:fs/promises";
import os from "node:os";
import path from "node:path";
import { METRICS_SCRIPT, MetricsTracker, parseSample, takeSamples } from "./remote-metrics.mjs";
import { SftpClient, isDirectory, isFile, isSymlink } from "./sftp-client.mjs";

const POLL_MS = 500;
const HANDSHAKE_MS = 20000;
const PROGRESS_MS = 150;
const CONCURRENT_TRANSFERS = 2;
const MAX_STDERR = 4096;
const MAX_METRICS_BUFFER = 256 * 1024;
const MAX_PATHS = 1000;
const MAX_DEPTH = 64;
const MAX_DRAG_BYTES = 2 * 1024 * 1024 * 1024;
// sockaddr_un is 104 bytes on macOS and ssh appends a 17 byte temporary suffix.
const MAX_CONTROL_PATH = 86;
const posix = path.posix;
const WINDOWS_RESERVED = /^(con|prn|aux|nul|com\d|lpt\d)(\.|$)/i;

// A server chooses the names it lists, so every name must stay inside the local
// destination directory no matter what the server sends.
export function safeLocalName(name, platform = process.platform) {
  if (typeof name !== "string" || !name || name === "." || name === ".." || name.length > 255 || name.includes("/") || name.includes("\0")) return false;
  if (platform === "win32" && (/[<>:"\\|?*\u0000-\u001f]/.test(name) || /[. ]$/.test(name) || WINDOWS_RESERVED.test(name))) return false;
  return true;
}

function remotePath(value, label = "Remote path") {
  if (typeof value !== "string" || !value || value.length > 4096 || value.includes("\0")) throw new Error(`${label} must be text of at most 4096 characters.`);
  return value;
}

function pathList(value, validate) {
  if (!Array.isArray(value) || !value.length || value.length > MAX_PATHS) throw new Error(`Choose between 1 and ${MAX_PATHS} items.`);
  return value.map((item) => validate(item));
}

function localPath(value) {
  if (typeof value !== "string" || value.length > 4096 || value.includes("\0") || !path.isAbsolute(value)) throw new Error("Local paths must be absolute.");
  return value;
}

function failureReason(stderr, fallback = "failed") {
  if (/subsystem request failed|subsystem.*(not|denied)/i.test(stderr)) return "no-sftp";
  if (/permission denied|host key verification failed|no supported authentication/i.test(stderr)) return "auth";
  return fallback;
}

export class RemoteManager {
  constructor({ spawnProcess = spawn, runSync = spawnSync, exists = existsSync, onState = () => {}, onTransfer = () => {}, onMetrics = () => {}, platform = process.platform, tempRoot = platform === "darwin" ? "/tmp" : os.tmpdir(), pollMs = POLL_MS, handshakeMs = HANDSHAKE_MS } = {}) {
    Object.assign(this, { spawnProcess, runSync, exists, onState, onTransfer, onMetrics, platform, tempRoot, pollMs, handshakeMs });
    this.remotes = new Map();
    this.controlDirectory = null;
    this.nextControl = 1;
  }
  // Called while planning a session. Returns null when ssh could not use the
  // path: an unusable ControlPath is a fatal ssh error, not a soft failure.
  controlPath() {
    if (this.platform === "win32") return null;
    // macOS's per-user TMPDIR may consume most of sockaddr_un before adding a
    // directory, socket name and OpenSSH's temporary suffix. Use its short
    // system /tmp by default, with an atomically created owner-only directory.
    try {
      if (!this.controlDirectory) {
        const directory = mkdtempSync(path.join(this.tempRoot, "omniterm-ssh-"));
        try { chmodSync(directory, 0o700); }
        catch (error) { rmSync(directory, { recursive: true, force: true }); throw error; }
        this.controlDirectory = directory;
      }
    }
    catch { return null; }
    const candidate = path.join(this.controlDirectory, String(this.nextControl++));
    return Buffer.byteLength(candidate) <= MAX_CONTROL_PATH && /^[\w./-]+$/.test(candidate) ? candidate : null;
  }
  channelArgs(target) {
    const control = target.controlPath ? ["-o", `ControlPath=${target.controlPath}`] : ["-o", "ControlPath=none"];
    return ["-o", "BatchMode=yes", "-o", "ControlMaster=no", ...control, "-o", "ClearAllForwardings=yes", "-o", "RemoteCommand=none", "-o", "RequestTTY=no", "-o", "ConnectTimeout=15", ...target.args];
  }
  require(id) {
    const remote = this.remotes.get(id);
    if (typeof id !== "string" || !remote) throw new Error("This session has no remote connection.");
    return remote;
  }
  ready(id) {
    const remote = this.require(id);
    if (remote.state !== "ready") throw new Error("The SFTP connection is not ready.");
    return remote;
  }
  describe(remote) {
    return { id: remote.id, state: remote.state, reason: remote.reason, detail: remote.detail, home: remote.home };
  }
  publish(remote, patch) {
    Object.assign(remote, patch);
    if (this.remotes.get(remote.id) === remote) this.onState(this.describe(remote));
  }
  state(id) {
    const remote = this.remotes.get(id);
    return remote ? this.describe(remote) : null;
  }
  attach(id, target) {
    if (this.remotes.has(id)) return;
    const remote = { id, target, state: "waiting", reason: null, detail: "", home: null, sftp: null, child: null, poll: null, linked: false, metricsWanted: false, metricsChild: null, transfers: new Map(), queue: [], running: 0, dragDirectory: null, dragFiles: new Map() };
    this.remotes.set(id, remote);
    this.publish(remote, {});
    if (target.controlPath) this.waitForLogin(remote);
    else this.linked(remote);
  }
  // The control socket only appears after authentication finishes, however long
  // the user spends on a host key prompt, password or second factor.
  waitForLogin(remote) {
    const check = () => {
      remote.poll = null;
      if (this.remotes.get(remote.id) !== remote) return;
      if (this.exists(remote.target.controlPath)) this.linked(remote);
      else remote.poll = setTimeout(check, this.pollMs);
    };
    check();
  }
  linked(remote) {
    remote.linked = true;
    if (remote.metricsWanted) this.startMetrics(remote);
    this.connect(remote);
  }
  async connect(remote) {
    this.publish(remote, { state: "connecting", reason: null, detail: "" });
    const { target } = remote;
    let stderr = "";
    let settled = false;
    let timer;
    const child = this.spawnProcess(target.executable, [...this.channelArgs(target), "-s", target.destination, "sftp"], { env: target.env, stdio: ["pipe", "pipe", "pipe"], shell: false, windowsHide: true });
    remote.child = child;
    const client = new SftpClient(child.stdin, child.stdout);
    child.stderr.on("data", (chunk) => { stderr = (stderr + chunk).slice(-MAX_STDERR); });
    const finish = (reason, detail) => {
      clearTimeout(timer);
      if (remote.child !== child) return;
      remote.child = null;
      remote.sftp = null;
      client.close();
      child.kill();
      for (const transfer of remote.transfers.values()) transfer.controller.abort();
      this.publish(remote, { state: "unavailable", reason, detail: detail.trim().slice(-500), home: null });
    };
    child.once("error", (error) => { settled = true; finish("failed", error.message); });
    child.once("close", (code) => {
      // Through a shared connection ssh prints nothing when the server refuses
      // the subsystem, because the refusal goes to the master process.
      const reason = settled ? (remote.state === "ready" ? "lost" : "failed") : failureReason(stderr, target.controlPath && !stderr.trim() ? "no-sftp" : "failed");
      settled = true;
      finish(reason, stderr || `ssh exited with code ${code}.`);
    });
    timer = setTimeout(() => { settled = true; finish("failed", "Timed out while opening the SFTP channel."); }, this.handshakeMs);
    try {
      await client.init();
      const home = await client.realpath(".");
      clearTimeout(timer);
      settled = true;
      if (remote.child !== child) return;
      remote.sftp = client;
      this.publish(remote, { state: "ready", home });
    } catch (error) {
      if (settled) return;
      settled = true;
      finish(failureReason(stderr), stderr || error.message);
    }
  }
  retry(id) {
    const remote = this.require(id);
    if (!remote.linked) return this.describe(remote);
    if (remote.state === "unavailable") this.connect(remote);
    if (remote.metricsWanted && !remote.metricsChild) this.startMetrics(remote);
    return this.describe(remote);
  }

  watchMetrics({ id, enabled } = {}) {
    const remote = this.require(id);
    remote.metricsWanted = Boolean(enabled);
    if (!enabled) this.stopMetrics(remote);
    else if (remote.linked && !remote.metricsChild) this.startMetrics(remote);
    return true;
  }
  startMetrics(remote) {
    const { target } = remote;
    const child = this.spawnProcess(target.executable, [...this.channelArgs(target), target.destination, `sh -c '${METRICS_SCRIPT}'`], { env: target.env, stdio: ["ignore", "pipe", "ignore"], shell: false, windowsHide: true });
    remote.metricsChild = child;
    const tracker = new MetricsTracker();
    let buffer = "";
    let delivered = false;
    child.stdout.setEncoding("utf8");
    child.stdout.on("data", (chunk) => {
      buffer += chunk;
      const { samples, rest } = takeSamples(buffer);
      buffer = rest;
      if (buffer.length > MAX_METRICS_BUFFER) { child.kill(); return; }
      for (const text of samples) {
        delivered = true;
        if (remote.metricsChild === child) this.onMetrics({ id: remote.id, available: true, sample: tracker.next(parseSample(text)) });
      }
    });
    child.once("error", () => {});
    child.once("close", () => {
      if (remote.metricsChild !== child) return;
      remote.metricsChild = null;
      this.onMetrics({ id: remote.id, available: false, delivered });
    });
  }
  stopMetrics(remote) {
    const child = remote.metricsChild;
    remote.metricsChild = null;
    child?.kill();
  }

  async list({ id, path: target } = {}) {
    const { sftp } = this.ready(id);
    const directory = await sftp.realpath(remotePath(target));
    const entries = await sftp.readdir(directory);
    const links = entries.filter((entry) => isSymlink(entry.attributes));
    // Symlinks report their own metadata; resolve them so linked folders open.
    await Promise.all(links.map(async (entry) => {
      try { entry.target = await sftp.stat(posix.join(directory, entry.name)); }
      catch { entry.target = null; }
    }));
    return {
      path: directory,
      entries: entries.map(({ name, attributes, target: linked }) => ({
        name,
        directory: isDirectory(linked ?? attributes),
        symlink: isSymlink(attributes),
        size: (linked ?? attributes).size ?? 0,
        mtime: (linked ?? attributes).mtime ?? 0,
        mode: attributes.mode ?? 0,
      })),
    };
  }
  async mkdir({ id, path: target } = {}) { await this.ready(id).sftp.mkdir(remotePath(target)); return true; }
  async rename({ id, from, to } = {}) { await this.ready(id).sftp.rename(remotePath(from), remotePath(to)); return true; }
  async remove({ id, paths } = {}) {
    const { sftp } = this.ready(id);
    for (const target of pathList(paths, remotePath)) {
      if (posix.normalize(target) === "/") throw new Error("The remote root directory cannot be deleted.");
      await this.removeTree(sftp, target, 0);
    }
    return true;
  }
  async removeTree(sftp, target, depth) {
    // lstat so a link to a directory removes the link and never what it points at.
    const attributes = await sftp.lstat(target);
    if (!isDirectory(attributes)) { await sftp.remove(target); return; }
    if (depth >= MAX_DEPTH) throw new Error("The remote directory is nested too deeply to delete.");
    for (const entry of await sftp.readdir(target)) await this.removeTree(sftp, posix.join(target, entry.name), depth + 1);
    await sftp.rmdir(target);
  }

  // Conflicts are reported before anything is queued so the user decides about
  // replacing data; `overwrite` is only sent after they confirm.
  async upload({ id, paths, directory, overwrite = false } = {}) {
    const remote = this.ready(id);
    const sources = pathList(paths, localPath);
    const destination = remotePath(directory, "Remote directory");
    const items = sources.map((source) => ({ source, name: path.basename(source), target: posix.join(destination, path.basename(source)) }));
    if (!overwrite) {
      const conflicts = [];
      for (const item of items) if (await remote.sftp.exists(item.target)) conflicts.push(item.name);
      if (conflicts.length) return { conflicts };
    }
    return { transfers: items.map((item) => this.enqueue(remote, "upload", item.name, (transfer) => this.runUpload(remote, transfer, item))) };
  }
  async download({ id, paths, destination, overwrite = false } = {}) {
    const remote = this.ready(id);
    const sources = pathList(paths, remotePath);
    const directory = localPath(destination);
    const items = sources.map((source) => {
      const name = posix.basename(source);
      if (!safeLocalName(name, this.platform)) throw new Error(`Refusing to download an unsafe file name: ${JSON.stringify(name)}`);
      return { source, name, target: path.join(directory, name) };
    });
    if (!overwrite) {
      const conflicts = [];
      for (const item of items) if (await lstat(item.target).then(() => true, () => false)) conflicts.push(item.name);
      if (conflicts.length) return { conflicts, destination: directory };
    }
    return { transfers: items.map((item) => this.enqueue(remote, "download", item.name, (transfer) => this.runDownload(remote, transfer, item))) };
  }
  // Native drags need a finished local file, so a dragged item is fetched into a
  // private temporary directory first and handed to the OS by the returned token.
  async prepareDrag({ id, path: source } = {}) {
    const remote = this.ready(id);
    const target = remotePath(source);
    const name = posix.basename(target);
    if (!safeLocalName(name, this.platform)) throw new Error(`Refusing to download an unsafe file name: ${JSON.stringify(name)}`);
    remote.dragDirectory ??= await mkdtemp(path.join(os.tmpdir(), "omniterm-drag-"));
    const directory = path.join(remote.dragDirectory, randomUUID());
    await mkdir(directory);
    const item = { source: target, name, target: path.join(directory, name), limit: MAX_DRAG_BYTES };
    const token = randomUUID();
    await new Promise((resolve, reject) => {
      this.enqueue(remote, "download", name, async (transfer) => {
        try { const skipped = await this.runDownload(remote, transfer, item); resolve(); return skipped; }
        catch (error) { reject(error); throw error; }
      });
    });
    remote.dragFiles.set(token, item.target);
    return { token };
  }
  dragFile({ id, token } = {}) {
    const file = this.require(id).dragFiles.get(token);
    if (!file) throw new Error("The prepared file is no longer available.");
    return file;
  }
  enqueue(remote, direction, name, run) {
    const transfer = { transferId: randomUUID(), direction, name, bytes: 0, total: 0, status: "queued", skipped: 0, error: "", controller: new AbortController(), run, lastEmit: 0 };
    remote.transfers.set(transfer.transferId, transfer);
    remote.queue.push(transfer);
    this.emitTransfer(remote, transfer);
    this.pump(remote);
    return transfer.transferId;
  }
  emitTransfer(remote, transfer) {
    const { transferId, direction, name, bytes, total, status, skipped, error } = transfer;
    transfer.lastEmit = Date.now();
    this.onTransfer({ id: remote.id, transferId, direction, name, bytes, total, status, skipped, error });
  }
  progress(remote, transfer, bytes) {
    transfer.bytes += bytes;
    if (Date.now() - transfer.lastEmit >= PROGRESS_MS) this.emitTransfer(remote, transfer);
  }
  pump(remote) {
    while (remote.running < CONCURRENT_TRANSFERS && remote.queue.length) {
      const transfer = remote.queue.shift();
      remote.running++;
      transfer.status = "running";
      this.emitTransfer(remote, transfer);
      transfer.run(transfer).then((skipped) => {
        transfer.skipped = skipped;
        this.settle(remote, transfer, "done");
      }, (error) => {
        if (transfer.controller.signal.aborted) this.settle(remote, transfer, "canceled");
        else { transfer.error = error.message || String(error); this.settle(remote, transfer, "failed"); }
      }).finally(() => { remote.running--; this.pump(remote); });
    }
  }
  settle(remote, transfer, status) {
    transfer.status = status;
    remote.transfers.delete(transfer.transferId);
    this.emitTransfer(remote, transfer);
  }
  cancel({ id, transferId } = {}) {
    const transfer = this.require(id).transfers.get(transferId);
    transfer?.controller.abort();
    return Boolean(transfer);
  }

  async runUpload(remote, transfer, item) {
    const { sftp } = this.ready(remote.id);
    const { signal } = transfer.controller;
    if (signal.aborted) throw new Error("Transfer canceled.");
    const plan = [];
    let skipped = 0;
    // Links are skipped below the top level; following them can loop forever or
    // send files from outside the folder the user chose.
    const walk = async (source, target, details, depth) => {
      if (depth > MAX_DEPTH) { skipped++; return; }
      if (details.isDirectory()) {
        plan.push({ directory: true, target });
        for (const entry of await readdir(source, { withFileTypes: true })) {
          if (!entry.isDirectory() && !entry.isFile()) { skipped++; continue; }
          const child = path.join(source, entry.name);
          await walk(child, posix.join(target, entry.name), await lstat(child), depth + 1);
        }
      } else if (details.isFile()) {
        plan.push({ source, target, size: details.size, mode: details.mode & 0o777 });
        transfer.total += details.size;
      } else skipped++;
    };
    await walk(item.source, item.target, await stat(item.source), 0);
    this.emitTransfer(remote, transfer);
    for (const step of plan) {
      if (signal.aborted) throw new Error("Transfer canceled.");
      if (step.directory) {
        const existing = await sftp.stat(step.target).catch(() => null);
        if (!existing) await sftp.mkdir(step.target);
        else if (!isDirectory(existing)) throw new Error(`A file already exists at ${step.target}.`);
        continue;
      }
      const partial = `${step.target}.omniterm-${randomUUID().slice(0, 8)}.part`;
      try {
        await sftp.upload(step.source, partial, { signal, mode: step.mode, onBytes: (bytes) => this.progress(remote, transfer, bytes) });
        await sftp.replace(partial, step.target);
      } catch (error) {
        await sftp.remove(partial).catch(() => {});
        throw error;
      }
    }
    return skipped;
  }
  async runDownload(remote, transfer, item) {
    const { sftp } = this.ready(remote.id);
    const { signal } = transfer.controller;
    if (signal.aborted) throw new Error("Transfer canceled.");
    const plan = [];
    let skipped = 0;
    const walk = async (source, target, attributes, depth) => {
      if (depth > MAX_DEPTH) { skipped++; return; }
      if (isDirectory(attributes)) {
        plan.push({ directory: true, target });
        for (const entry of await sftp.readdir(source)) {
          if (!safeLocalName(entry.name, this.platform) || (!isDirectory(entry.attributes) && !isFile(entry.attributes))) { skipped++; continue; }
          await walk(posix.join(source, entry.name), path.join(target, entry.name), entry.attributes, depth + 1);
        }
      } else if (isFile(attributes)) {
        plan.push({ source, target, size: attributes.size ?? 0, mode: (attributes.mode ?? 0o644) & 0o777 });
        transfer.total += attributes.size ?? 0;
      } else skipped++;
    };
    await walk(item.source, item.target, await sftp.stat(item.source), 0);
    if (item.limit && transfer.total > item.limit) throw new Error("This item is too large to drag out. Use Download instead.");
    this.emitTransfer(remote, transfer);
    for (const step of plan) {
      if (signal.aborted) throw new Error("Transfer canceled.");
      if (step.directory) { await mkdir(step.target, { recursive: true }); continue; }
      const partial = path.join(path.dirname(step.target), `.${randomUUID().slice(0, 8)}.omniterm-part`);
      try {
        await sftp.download(step.source, partial, { signal, mode: step.mode, onBytes: (bytes) => this.progress(remote, transfer, bytes) });
        await rename(partial, step.target);
      } catch (error) {
        await rm(partial, { force: true });
        throw error;
      }
    }
    return skipped;
  }

  detach(id, { sync = false } = {}) {
    const remote = this.remotes.get(id);
    if (!remote) return;
    this.remotes.delete(id);
    clearTimeout(remote.poll);
    for (const transfer of remote.transfers.values()) transfer.controller.abort();
    this.stopMetrics(remote);
    const child = remote.child;
    remote.child = null;
    remote.sftp = null;
    child?.stdin.end();
    child?.kill();
    if (remote.dragDirectory) rmSync(remote.dragDirectory, { recursive: true, force: true });
    this.onState({ id, state: "closed", reason: null, detail: "", home: null });
    const { target } = remote;
    if (!target.controlPath || !this.exists(target.controlPath)) return;
    // The master outlives the terminal for ControlPersist seconds; end it now.
    const args = ["-o", `ControlPath=${target.controlPath}`, "-O", "exit", target.destination];
    if (sync) this.runSync(target.executable, args, { env: target.env, stdio: "ignore", timeout: 1500, windowsHide: true });
    else this.spawnProcess(target.executable, args, { env: target.env, stdio: "ignore", shell: false, windowsHide: true }).once("error", () => {});
  }
  dispose() {
    for (const id of [...this.remotes.keys()]) this.detach(id, { sync: true });
    if (this.controlDirectory) rmSync(this.controlDirectory, { recursive: true, force: true });
    this.controlDirectory = null;
  }
}
