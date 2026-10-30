import test from "node:test";
import assert from "node:assert/strict";
import { EventEmitter } from "node:events";
import { PassThrough } from "node:stream";
import { FrameDecoder, PROTOCOL_VERSION, TerminalHost, resolveTerminalHost } from "./terminal-host.mjs";

const jsonFrame = (value) => {
  const body = Buffer.from(JSON.stringify(value));
  const frame = Buffer.alloc(5 + body.length);
  frame.writeUInt32LE(body.length + 1, 0);
  frame[4] = 1;
  body.copy(frame, 5);
  return frame;
};

const dataFrame = (id, seq, text) => {
  const bytes = Buffer.from(text);
  const frame = Buffer.alloc(4 + 2 + id.length + 8 + bytes.length);
  frame.writeUInt32LE(frame.length - 4, 0);
  frame[4] = 2;
  frame[5] = id.length;
  frame.write(id, 6);
  frame.writeBigUInt64LE(BigInt(seq), 6 + id.length);
  bytes.copy(frame, 14 + id.length);
  return frame;
};

function fakeChild() {
  const child = new EventEmitter();
  child.stdout = new PassThrough();
  child.stdin = new PassThrough();
  child.killed = false;
  child.kill = () => { child.killed = true; };
  return child;
}

test("frames decode whatever way the stream splits them", () => {
  const stream = Buffer.concat([jsonFrame({ id: 1, result: "ok" }), dataFrame("abc", 9, "中文"), jsonFrame({ event: "e" })]);
  for (let size = 1; size <= 7; size++) {
    const decoder = new FrameDecoder();
    const frames = [];
    for (let offset = 0; offset < stream.length; offset += size) frames.push(...decoder.push(stream.subarray(offset, offset + size)));
    assert.deepEqual(frames, [{ json: { id: 1, result: "ok" } }, { data: { id: "abc", seq: 9, data: "中文" } }, { json: { event: "e" } }]);
  }
});

test("a corrupt frame length is refused instead of buffering forever", () => {
  const frame = Buffer.alloc(8);
  frame.writeUInt32LE(0xffffffff, 0);
  assert.throws(() => new FrameDecoder().push(frame), /corrupt/);
});

test("responses resolve by id, errors reject, and events and data reach their handlers", async () => {
  const child = fakeChild();
  const events = [];
  const data = [];
  const host = new TerminalHost({ executable: "host", spawnProcess: () => child, onEvent: (...event) => events.push(event), onData: (item) => data.push(item) });
  const written = [];
  child.stdin.on("data", (chunk) => written.push(JSON.parse(chunk)));
  const first = host.request("a", { x: 1 });
  const second = host.request("b");
  child.stdout.write(Buffer.concat([jsonFrame({ event: "host.ready", params: { protocolVersion: PROTOCOL_VERSION } }), jsonFrame({ id: 2, error: { message: "nope" } }), jsonFrame({ id: 1, result: 5 }), dataFrame("s", 1, "x")]));
  assert.equal(await first, 5);
  await assert.rejects(second, /nope/);
  assert.deepEqual(written.map(({ method }) => method), ["a", "b"]);
  assert.equal(events[0][0], "host.ready");
  assert.deepEqual(data, [{ id: "s", seq: 1, data: "x" }]);
});

test("a host from another build is rejected before it is used", async () => {
  const child = fakeChild();
  const host = new TerminalHost({ executable: "host", spawnProcess: () => child });
  const request = host.request("a");
  child.stdout.write(jsonFrame({ event: "host.ready", params: { protocolVersion: PROTOCOL_VERSION + 1 } }));
  await assert.rejects(request, /does not match/);
  assert.equal(child.killed, true);
});

test("a stopped host fails waiting requests and reports the loss", async () => {
  const child = fakeChild();
  let lost = 0;
  const host = new TerminalHost({ executable: "host", spawnProcess: () => child, onExit: () => lost++ });
  const request = host.request("a");
  child.emit("exit", 1);
  await assert.rejects(request, /stopped/);
  assert.equal(lost, 1);
});

test("the host starts again after a failure, but never after disposal", async () => {
  const children = [fakeChild(), fakeChild()];
  let spawned = 0;
  const host = new TerminalHost({ executable: "host", spawnProcess: () => children[spawned++] });
  host.request("a").catch(() => {});
  children[0].emit("exit", 1);
  host.request("b").catch(() => {});
  assert.equal(spawned, 2);
  host.dispose();
  await assert.rejects(host.request("c"), /closing/);
  assert.equal(spawned, 2);
});

test("packaged builds only run the bundled host", () => {
  const options = { packaged: true, resourcesPath: "/app/resources", desktopRoot: "/src", platform: "linux", env: { OMNITERM_TERMINAL_HOST: "/tmp/other" } };
  assert.equal(resolveTerminalHost({ ...options, exists: (name) => name === "/app/resources/terminal-host/omniterm-terminal-host" }), "/app/resources/terminal-host/omniterm-terminal-host");
  assert.throws(() => resolveTerminalHost({ ...options, exists: (name) => name === "/tmp/other" }), /missing/);
  assert.equal(resolveTerminalHost({ desktopRoot: "/src", platform: "linux", env: {}, exists: (name) => name.endsWith("target/debug/omniterm-terminal-host") }), "/src/native/terminal-host/target/debug/omniterm-terminal-host");
  assert.throws(() => resolveTerminalHost({ desktopRoot: "/src", platform: "linux", env: {}, exists: () => false }), /build:native/);
});
