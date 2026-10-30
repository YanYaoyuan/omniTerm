// Copyright 2026 omniTerm contributors. SPDX-License-Identifier: Apache-2.0
// A small SFTP version 3 client over any byte stream pair. The desktop app runs
// it over `ssh -s sftp`, so OpenSSH keeps handling keys, host verification and
// connection sharing while file names never pass through a shell or `ls` output.
import { open as openLocal } from "node:fs/promises";

const INIT = 1, VERSION = 2, OPEN = 3, CLOSE = 4, READ = 5, WRITE = 6, LSTAT = 7, OPENDIR = 11, READDIR = 12, REMOVE = 13, MKDIR = 14, RMDIR = 15, REALPATH = 16, STAT = 17, RENAME = 18, READLINK = 19, EXTENDED = 200;
const STATUS = 101, HANDLE = 102, DATA = 103, NAME = 104, ATTRS = 105, EXTENDED_REPLY = 201;
const STATUS_OK = 0, STATUS_EOF = 1;
const ATTR_SIZE = 1, ATTR_UIDGID = 2, ATTR_PERMISSIONS = 4, ATTR_TIMES = 8, ATTR_EXTENDED = 0x80000000;
const OPEN_READ = 1, OPEN_WRITE = 2, OPEN_CREATE = 8, OPEN_TRUNCATE = 0x10;
const MAX_PACKET = 1024 * 1024;
const MAX_ENTRIES = 100000;
// 32 KiB is the largest payload the protocol requires every server to accept.
const CHUNK = 32 * 1024;
const WINDOW = 16;

export const S_IFMT = 0o170000;
export const S_IFDIR = 0o040000;
export const S_IFREG = 0o100000;
export const S_IFLNK = 0o120000;
export const NO_SUCH_FILE = 2;
export const PERMISSION_DENIED = 3;

export class SftpError extends Error {
  constructor(code, message) {
    super(message || `SFTP request failed (status ${code}).`);
    this.name = "SftpError";
    this.code = code;
  }
}

class Reader {
  constructor(buffer) { this.buffer = buffer; this.offset = 0; }
  need(size) { if (this.offset + size > this.buffer.length) throw new Error("The SFTP server sent a truncated packet."); }
  u8() { this.need(1); return this.buffer[this.offset++]; }
  u32() { this.need(4); const value = this.buffer.readUInt32BE(this.offset); this.offset += 4; return value; }
  u64() { this.need(8); const value = this.buffer.readBigUInt64BE(this.offset); this.offset += 8; return Number(value); }
  bytes() { const size = this.u32(); this.need(size); const value = this.buffer.subarray(this.offset, this.offset + size); this.offset += size; return value; }
  text() { return this.bytes().toString("utf8"); }
  attributes() {
    const flags = this.u32();
    const result = {};
    if (flags & ATTR_SIZE) result.size = this.u64();
    if (flags & ATTR_UIDGID) { this.u32(); this.u32(); }
    if (flags & ATTR_PERMISSIONS) result.mode = this.u32();
    if (flags & ATTR_TIMES) { this.u32(); result.mtime = this.u32(); }
    if (flags & ATTR_EXTENDED) for (let count = this.u32(); count > 0; count--) { this.bytes(); this.bytes(); }
    return result;
  }
}

const u32 = (value) => { const buffer = Buffer.alloc(4); buffer.writeUInt32BE(value); return buffer; };
const u64 = (value) => { const buffer = Buffer.alloc(8); buffer.writeBigUInt64BE(BigInt(value)); return buffer; };
const string = (value) => { const buffer = Buffer.isBuffer(value) ? value : Buffer.from(value, "utf8"); return Buffer.concat([u32(buffer.length), buffer]); };
const modeAttributes = (mode) => mode === undefined ? u32(0) : Buffer.concat([u32(ATTR_PERMISSIONS), u32(mode)]);
const aborted = (signal) => { if (signal?.aborted) { const error = new Error("Transfer canceled."); error.name = "AbortError"; throw error; } };

export const isDirectory = (attributes) => (attributes.mode & S_IFMT) === S_IFDIR;
export const isFile = (attributes) => (attributes.mode & S_IFMT) === S_IFREG;
export const isSymlink = (attributes) => (attributes.mode & S_IFMT) === S_IFLNK;

export class SftpClient {
  constructor(input, output) {
    this.input = input;
    this.output = output;
    this.nextId = 1;
    this.pending = new Map();
    this.chunks = [];
    this.buffered = 0;
    this.closed = null;
    this.extensions = new Map();
    this.onData = (chunk) => this.receive(chunk);
    output.on("data", this.onData);
    // Writing to a dead ssh surfaces as EPIPE on stdin; the pending requests are
    // rejected by whoever notices the exit, and must not crash the main process.
    input.on("error", (error) => this.fail(error));
  }
  async init() {
    const reply = new Promise((resolve, reject) => { this.pending.set(0, { resolve, reject }); });
    this.send(Buffer.concat([Buffer.from([INIT]), u32(3)]));
    const reader = await reply;
    const version = reader.u32();
    if (version < 3) throw new Error(`The SFTP server only supports protocol version ${version}.`);
    while (reader.offset < reader.buffer.length) this.extensions.set(reader.text(), reader.text());
    return this;
  }
  send(body) {
    if (this.closed) throw this.closed;
    this.input.write(Buffer.concat([u32(body.length), body]));
  }
  fail(error) {
    this.closed ??= error;
    const pending = [...this.pending.values()];
    this.pending.clear();
    for (const request of pending) request.reject(this.closed);
  }
  close(reason = new Error("The SFTP connection was closed.")) {
    this.output.off("data", this.onData);
    this.fail(reason);
    try { this.input.end(); } catch { /* The stream may already be gone. */ }
  }
  receive(chunk) {
    this.chunks.push(chunk);
    this.buffered += chunk.length;
    if (this.buffered < 4) return;
    let buffer = this.chunks.length === 1 ? this.chunks[0] : Buffer.concat(this.chunks, this.buffered);
    let offset = 0;
    try {
      while (buffer.length - offset >= 4) {
        const size = buffer.readUInt32BE(offset);
        if (size < 1 || size > MAX_PACKET) throw new Error("The SFTP server sent an invalid packet size.");
        if (buffer.length - offset - 4 < size) break;
        this.dispatch(buffer.subarray(offset + 4, offset + 4 + size));
        offset += 4 + size;
      }
    } catch (error) { this.close(error); return; }
    buffer = buffer.subarray(offset);
    this.chunks = buffer.length ? [buffer] : [];
    this.buffered = buffer.length;
  }
  dispatch(packet) {
    const reader = new Reader(packet);
    const type = reader.u8();
    if (type === VERSION) {
      const request = this.pending.get(0);
      this.pending.delete(0);
      request?.resolve(reader);
      return;
    }
    const id = reader.u32();
    const request = this.pending.get(id);
    if (!request) return;
    this.pending.delete(id);
    if (type === STATUS) {
      const code = reader.u32();
      const message = reader.offset < packet.length ? reader.text() : "";
      if (code === STATUS_OK || (code === STATUS_EOF && request.allowEof)) request.resolve(null);
      else request.reject(new SftpError(code, message));
      return;
    }
    if (![HANDLE, DATA, NAME, ATTRS, EXTENDED_REPLY].includes(type)) { request.reject(new Error(`Unexpected SFTP reply type ${type}.`)); return; }
    try {
      // Copied so a small handle or chunk does not pin the whole receive buffer.
      if (type === HANDLE || type === DATA) request.resolve(Buffer.from(reader.bytes()));
      else if (type === ATTRS) request.resolve(reader.attributes());
      else if (type === NAME) {
        const count = reader.u32();
        if (count > MAX_ENTRIES) throw new Error("The SFTP server sent too many directory entries.");
        const names = [];
        for (let index = 0; index < count; index++) names.push({ name: reader.text(), longname: reader.text(), attributes: reader.attributes() });
        request.resolve(names);
      } else request.resolve(reader);
    } catch (error) { request.reject(error); }
  }
  request(type, parts, { allowEof = false } = {}) {
    const id = this.nextId++;
    return new Promise((resolve, reject) => {
      this.pending.set(id, { resolve, reject, allowEof });
      try { this.send(Buffer.concat([Buffer.from([type]), u32(id), ...parts])); }
      catch (error) { this.pending.delete(id); reject(error); }
    });
  }
  async realpath(target) { return (await this.request(REALPATH, [string(target)]))[0].name; }
  stat(target) { return this.request(STAT, [string(target)]); }
  lstat(target) { return this.request(LSTAT, [string(target)]); }
  async exists(target) {
    try { await this.lstat(target); return true; }
    catch (error) { if (error.code === NO_SUCH_FILE) return false; throw error; }
  }
  mkdir(target) { return this.request(MKDIR, [string(target), modeAttributes(0o755)]); }
  rmdir(target) { return this.request(RMDIR, [string(target)]); }
  remove(target) { return this.request(REMOVE, [string(target)]); }
  rename(from, to) { return this.request(RENAME, [string(from), string(to)]); }
  async readlink(target) { return (await this.request(READLINK, [string(target)]))[0].name; }
  // Replaces the destination atomically where the server allows it. Plain SFTP
  // v3 rename refuses an existing destination, so the fallback removes it first.
  async replace(from, to) {
    if (this.extensions.has("posix-rename@openssh.com")) { await this.request(EXTENDED, [string("posix-rename@openssh.com"), string(from), string(to)]); return; }
    if (await this.exists(to)) await this.remove(to);
    await this.rename(from, to);
  }
  async readdir(target) {
    const handle = await this.request(OPENDIR, [string(target)]);
    const entries = [];
    try {
      for (;;) {
        const batch = await this.request(READDIR, [string(handle)], { allowEof: true });
        if (!batch) break;
        for (const entry of batch) if (entry.name !== "." && entry.name !== "..") entries.push({ name: entry.name, attributes: entry.attributes });
        if (entries.length > MAX_ENTRIES) throw new Error("The remote directory has too many entries to list.");
      }
    } finally { await this.request(CLOSE, [string(handle)]).catch(() => {}); }
    return entries;
  }
  openFile(target, flags, mode) { return this.request(OPEN, [string(target), u32(flags), modeAttributes(mode)]); }
  // Requests overlap on the connection; each worker finishes its own chunk so a
  // short read from the server never leaves a hole in the local file.
  async download(remote, local, { signal, onBytes = () => {}, mode } = {}) {
    const handle = await this.openFile(remote, OPEN_READ);
    let file;
    try {
      file = await openLocal(local, "wx", mode);
      let next = 0;
      let eof = false;
      const worker = async () => {
        while (!eof) {
          aborted(signal);
          const start = next;
          next += CHUNK;
          let offset = start;
          while (offset < start + CHUNK) {
            const data = await this.request(READ, [string(handle), u64(offset), u32(start + CHUNK - offset)], { allowEof: true });
            if (!data || !data.length) { eof = true; return; }
            await file.write(data, 0, data.length, offset);
            offset += data.length;
            onBytes(data.length);
          }
        }
      };
      await Promise.all(Array.from({ length: WINDOW }, worker));
    } finally {
      await this.request(CLOSE, [string(handle)]).catch(() => {});
      await file?.close();
    }
  }
  async upload(local, remote, { signal, onBytes = () => {}, mode } = {}) {
    const file = await openLocal(local, "r");
    let handle;
    try {
      handle = await this.openFile(remote, OPEN_WRITE | OPEN_CREATE | OPEN_TRUNCATE, mode);
      let next = 0;
      const worker = async () => {
        const buffer = Buffer.alloc(CHUNK);
        for (;;) {
          aborted(signal);
          const offset = next;
          next += CHUNK;
          const { bytesRead } = await file.read(buffer, 0, CHUNK, offset);
          if (!bytesRead) return;
          await this.request(WRITE, [string(handle), u64(offset), string(buffer.subarray(0, bytesRead))]);
          onBytes(bytesRead);
          if (bytesRead < CHUNK) return;
        }
      };
      await Promise.all(Array.from({ length: WINDOW }, worker));
    } finally {
      if (handle) await this.request(CLOSE, [string(handle)]).catch(() => {});
      await file.close();
    }
  }
}
