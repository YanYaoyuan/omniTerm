// Copyright 2026 omniTerm contributors. SPDX-License-Identifier: Apache-2.0
import assert from "node:assert/strict";
import { mkdtemp, writeFile, rm } from "node:fs/promises";
import net from "node:net";
import os from "node:os";
import path from "node:path";
import test from "node:test";
import { discoveryDirectory, requestBridge, runSession } from "../src/sessions.js";
import { main } from "../src/cli.js";

async function fixture(t, handler = () => ({}), instance = "fixture-one") {
  const directory = await mkdtemp(path.join(os.tmpdir(), "omniterm-cli-test-"));
  const calls = [];
  const sockets = new Set();
  const token = "private-test-token";
  const server = net.createServer((socket) => {
    sockets.add(socket);
    socket.on("close", () => sockets.delete(socket));
    let buffer = "";
    socket.on("data", (chunk) => {
      buffer += chunk;
      if (!buffer.includes("\n")) return;
      const request = JSON.parse(buffer.slice(0, buffer.indexOf("\n")));
      calls.push(request);
      const response = request.token !== token ? { ok: false, error: "Authentication failed" }
        : { ok: true, result: request.method === "ping" ? { pid: process.pid, protocol: 1 } : handler(request.method, request.params) };
      socket.end(JSON.stringify(response) + "\n");
    });
  });
  await new Promise((resolve) => server.listen(0, "127.0.0.1", resolve));
  const descriptor = { protocol: 1, instance, pid: process.pid, port: server.address().port, token };
  await writeFile(path.join(directory, instance + ".json"), JSON.stringify(descriptor));
  t.after(async () => {
    for (const socket of sockets) socket.destroy();
    await new Promise((resolve) => server.close(resolve));
    await rm(directory, { recursive: true, force: true });
  });
  const run = async (args) => {
    let stdout = "", stderr = "";
    const code = await main(["session", ...args], { env: { OMNITERM_AUTOMATION_DIR: directory }, output: (s) => { stdout += s; }, errorOutput: (s) => { stderr += s; } });
    return { code, stdout, stderr, value: JSON.parse(stdout || stderr) };
  };
  return { descriptor, directory, calls, run };
}

test("native discovery follows platform data directory conventions", () => {
  assert.equal(discoveryDirectory({ HOME: "/u" }, "linux"), "/u/.config/omniTerm/automation");
  assert.equal(discoveryDirectory({ HOME: "/u", XDG_CONFIG_HOME: "/config" }, "linux"), "/config/omniTerm/automation");
  assert.equal(discoveryDirectory({ HOME: "/u", XDG_CONFIG_HOME: "relative" }, "linux"), "/u/.config/omniTerm/automation");
  assert.equal(discoveryDirectory({ HOME: "/u" }, "darwin"), "/u/Library/Application Support/omniTerm/automation");
  assert.match(discoveryDirectory({ APPDATA: "/roaming" }, "win32"), /omniTerm.*automation$/);
});

test("instances never expose tokens; malformed and stale discovery files are skipped", async (t) => {
  const f = await fixture(t);
  await writeFile(path.join(f.directory, "broken.json"), "not json");
  await writeFile(path.join(f.directory, "stale.json"), JSON.stringify({ ...f.descriptor, pid: -1 }));
  const result = await f.run(["instances"]);
  assert.equal(result.code, 0);
  assert.deepEqual(result.value.instances, [{ instance: "fixture-one", pid: process.pid }]);
  assert.ok(!result.stdout.includes(f.descriptor.token));
  await assert.rejects(requestBridge({ ...f.descriptor, token: "wrong" }, "list"), /Authentication failed/);
});

test("multiple native instances require explicit selection", async (t) => {
  const one = await fixture(t, () => ({ terminals: [] }));
  const two = await fixture(t, () => ({ terminals: [{ label: "second" }] }), "fixture-two");
  await writeFile(path.join(one.directory, "two.json"), JSON.stringify(two.descriptor));
  assert.equal((await one.run(["list"])).code, 1);
  const selected = await one.run(["list", "--instance", "fixture-two"]);
  assert.equal(selected.code, 0);
  assert.equal(selected.value.terminals[0].label, "second");
  assert.ok(!one.calls.some((call) => call.method === "list"));
});

test("exec sends literal command once then polls result for exit code", async (t) => {
  let reads = 0;
  const f = await fixture(t, (method) => method === "exec" ? { operation: { status: "running" } } : { operation: { status: ++reads < 2 ? "running" : "completed", exitCode: 7 }, output: { lines: ["result"] } });
  const command = "printf '%s\\n' '$HOME; $(echo unsafe)'\nfalse";
  const result = await f.run(["exec", "abcdefgh", "--epoch", "3", "--request-id", "fixed", "--command", command, "--wait-ms", "2000"]);
  assert.equal(result.code, 3);
  const exec = f.calls.filter((call) => call.method === "exec");
  assert.equal(exec.length, 1);
  assert.deepEqual(exec[0].params, { terminal: "abcdefgh", epoch: 3, requestId: "fixed", command, timeoutMs: 120000 });
  assert.equal(result.value.requestId, "fixed");
  assert.equal(result.value.output.lines[0], "result");
});

test("pending and unknown execution are distinct from success; result can recover", async (t) => {
  let status = "running";
  const f = await fixture(t, () => ({ operation: { status, exitCode: status === "completed" ? 0 : null } }));
  const result = await f.run(["exec", "abcdefgh", "--epoch", "1", "--command", "sleep 5", "--wait-ms", "0"]);
  assert.equal(result.code, 2);
  assert.equal(typeof result.value.requestId, "string");
  status = "outcomeUnknown";
  assert.equal((await f.run(["result", result.value.requestId])).code, 2);
  status = "completed";
  assert.equal((await f.run(["result", result.value.requestId])).code, 0);
});

test("read carries sequence cursor; raw input reports delivery without completion", async (t) => {
  const f = await fixture(t, (method) => method === "read" ? { lines: ["new"], seqEnd: 45, missingBytes: 0 } : { status: "sent", completionKnown: false });
  const read = await f.run(["read", "abcdefgh", "--epoch", "2", "--since", "42", "--lines", "100"]);
  assert.equal(read.value.seqEnd, 45);
  assert.deepEqual(f.calls.find((call) => call.method === "read").params, { terminal: "abcdefgh", epoch: 2, sinceSeq: 42, maxLines: 100 });
  const write = await f.run(["write", "abcdefgh", "--epoch", "2", "--text", "hello", "--enter"]);
  assert.equal(write.code, 0);
  assert.equal(write.value.completionKnown, false);
  assert.equal(f.calls.find((call) => call.method === "write").params.enter, true);
});

test("invalid options and missing epoch fail before opening a connection", async () => {
  const failures = [
    ["exec", "abcdefgh", "--command", "ls"],
    ["read", "abcdefgh", "--epoch", "NaN"],
    ["list", "--command", "ls"],
    ["exec", "abcdefgh", "--epoch", "1", "--command", "ls", "--wait-ms", "-1"],
    ["write", "abcdefgh", "--epoch", "1", "--unknown"],
    ["setup", "abcdefgh", "--epoch", "1", "--shell", "fish"],
    ["read", "abcdefgh", "--epoch", "1", "--epoch", "2"],
  ];
  for (const args of failures) {
    let output = "";
    const code = await runSession(args, { env: { OMNITERM_AUTOMATION_DIR: "/does-not-exist" }, errorOutput: (s) => { output += s; } });
    assert.equal(code, 1);
    assert.ok(!JSON.parse(output).error.includes("No matching"), output);
  }
});

test("truncated network response rejects instead of claiming a successful write", async (t) => {
  const server = net.createServer((socket) => socket.once("data", () => socket.end('{"ok":true')));
  await new Promise((resolve) => server.listen(0, "127.0.0.1", resolve));
  t.after(() => new Promise((resolve) => server.close(resolve)));
  await assert.rejects(requestBridge({ protocol: 1, port: server.address().port, token: "test" }, "exec"), /disconnected/);
});

test("file payload preserves multiline text without local shell evaluation", async (t) => {
  const f = await fixture(t, () => ({ operation: { status: "completed", exitCode: 0 } }));
  const payload = "printf '%s\\n' '$HOME; $(unsafe)'\nfalse\n";
  const filename = path.join(f.directory, "command with spaces.sh");
  await writeFile(filename, payload);
  const result = await f.run(["exec", "abcdefgh", "--epoch", "1", "--command-file", filename]);
  assert.equal(result.code, 0);
  assert.equal(f.calls.find((call) => call.method === "exec").params.command, payload);
  const invalid = await f.run(["exec", "abcdefgh", "--epoch", "1", "--command-file", filename, "--command", "ls"]);
  assert.equal(invalid.code, 1);
});
