// Live acceptance against a real model service. It spends tokens and needs a built engine and
// terminal host, so it is run by hand and never by CI. Commands only print random markers.
//
//   OMNITERM_LIVE_PROVIDER   qwen | anthropic | kimi | openrouter | openai | custom
//   OMNITERM_LIVE_MODEL      model ID
//   OMNITERM_LIVE_API_KEY    API key (read from the environment, never printed)
//   OMNITERM_LIVE_BASE_URL   optional; replaces the preset's URL (required for custom)
//   OMNITERM_LIVE_WIRE_API   optional; responses | chat | anthropic_messages
//   OMNITERM_LIVE_SSH_HOST   optional; a test host or alias reachable without a password prompt
import assert from "node:assert/strict";
import { randomBytes } from "node:crypto";
import { mkdtempSync, rmSync } from "node:fs";
import os from "node:os";
import path from "node:path";
import { fileURLToPath } from "node:url";
import { AgentService } from "../electron/agent-service.mjs";
import { buildCommandPlan, resolveDesktopAgent } from "../electron/command-plan.mjs";
import { PROVIDER_PRESETS, validateProviderProfile } from "../electron/provider-store.mjs";
import { SessionManager } from "../electron/session-manager.mjs";
import { shellIntegrationScript } from "../electron/shell-integration.mjs";
import { TerminalHost, resolveTerminalHost } from "../electron/terminal-host.mjs";
import { operationsIn, runTurn, settleTurn } from "./live-harness.mjs";
import { CURSOR_QUERY, CURSOR_REPLY, hasOutputLine, printParts } from "./pty-output.mjs";

const { OMNITERM_LIVE_PROVIDER: presetId, OMNITERM_LIVE_MODEL: model, OMNITERM_LIVE_API_KEY: apiKey, OMNITERM_LIVE_BASE_URL: baseUrl, OMNITERM_LIVE_WIRE_API: wireApi, OMNITERM_LIVE_SSH_HOST: sshHost } = process.env;
const preset = PROVIDER_PRESETS[presetId];
if (!preset || !model || !apiKey) { console.error("Set OMNITERM_LIVE_PROVIDER, OMNITERM_LIVE_MODEL and OMNITERM_LIVE_API_KEY. See the top of this file."); process.exit(2); }

const desktopRoot = path.resolve(path.dirname(fileURLToPath(import.meta.url)), "..");
const repositoryRoot = path.resolve(desktopRoot, "..");
const windows = process.platform === "win32";
const env = { ...process.env };
for (const name of Object.keys(env)) if (name.startsWith("OMNITERM_LIVE_")) delete env[name];
const profile = validateProviderProfile({ presetId, name: preset.label, wireApi: wireApi || preset.wireApi, baseUrl: baseUrl || preset.baseUrl, model });
const agentHome = mkdtempSync(path.join(os.tmpdir(), "omniterm-live-"));
const planOptions = { repositoryRoot, defaultCwd: repositoryRoot, env };
const delay = (ms) => new Promise((resolve) => setTimeout(resolve, ms));
const outputs = new Map();
const events = [];
const waiters = new Set();
const notify = () => { for (const check of [...waiters]) check(); };
function until(predicate, label, timeoutMs = 120_000) {
  return new Promise((resolve, reject) => {
    const check = () => { const value = predicate(); if (value) { clearTimeout(timer); waiters.delete(check); resolve(value); } };
    const timer = setTimeout(() => { waiters.delete(check); reject(new Error(`Timed out: ${label}`)); }, timeoutMs);
    waiters.add(check);
    check();
  });
}
let agent;
const host = new TerminalHost({
  executable: resolveTerminalHost({ desktopRoot, env }),
  onData: ({ id, data }) => {
    outputs.set(id, (outputs.get(id) ?? "") + data);
    if (data.includes(CURSOR_QUERY)) host.request("session.write", { id, data: CURSOR_REPLY, reply: true }).catch(() => {});
    notify();
  },
  onEvent: (event, params) => { if (event === "operation.update") agent.operationChanged(params); },
});
const manager = new SessionManager({ host, planner: (request) => buildCommandPlan(request, planOptions) });
agent = new AgentService({
  host, env, agentHome, clientVersion: "live-test",
  providers: { get: () => profile, recordVerification: () => [] },
  vault: { get: () => apiKey },
  resolveCommand: () => resolveDesktopAgent(planOptions),
  emit: (event) => { events.push(event); notify(); },
});

const open = async (request) => {
  const session = await manager.create(request);
  await manager.start(session.id);
  await delay(1500);
  return session;
};
const reference = (session) => ({ sessionId: session.id, epoch: 1 });
const report = (event) => { events.push(event); notify(); };
const turn = (text, context, targets, onOperation) => runTurn({ agent, events, until, report, onOperation, request: { profileId: profile.id, text, context: context.map(reference), targets: targets.map(reference), selections: {}, cwd: repositoryRoot } });
const results = [];
async function step(name, run) {
  const start = events.length;
  try { await run(); results.push(true); console.log(`PASS  ${name}`); }
  catch (error) {
    results.push(false);
    console.log(`FAIL  ${name}: ${error.message}`);
    // One failed check must leave nothing running for the next one to trip over.
    await settleTurn({ agent, events, until, start });
  }
}
const token = () => randomBytes(4).toString("hex").toUpperCase();
const statusOf = async (operationId) => (await host.request("operation.result", { operationId })).operation;

try {
  const local = await open({ kind: "shell" });
  if (!windows) { await manager.write({ id: local.id, data: `${shellIntegrationScript(manager.shellNonce(local.id))}\r` }); await delay(800); }
  const short = local.id.slice(0, 8);
  const interaction = async () => (await host.request("workspace.list")).terminals.find((terminal) => terminal.ref.sessionId === local.id).interaction.state;
  const localMode = async () => await interaction() === "ready" ? "run" : "stage";

  await step("streaming and two consecutive tool rounds", async () => {
    const { finished, events: seen } = await turn(`Call workspace_list_terminals. Then call terminal_read_context with terminal ${short}. Then reply with the single word DONE.`, [local], []);
    assert.equal(finished.type, "turn-completed", finished.message ?? finished.error);
    assert.equal(finished.error, null);
    const tools = new Set(seen.filter(({ type, status }) => type === "tool" && status === "completed").map(({ tool }) => tool));
    assert.ok(tools.has("workspace_list_terminals") && tools.has("terminal_read_context"), `tools used: ${[...tools].join(", ") || "none"}`);
    assert.ok(seen.some(({ type }) => type === "text-delta"), "no streamed text arrived");
  });

  await step("a denied command is not run, and the model is told", async () => {
    const mark = token();
    const { finished, events: seen } = await turn(`Propose and then execute this exact command in terminal ${short}: ${printParts(["__DENIED_", `${mark}__`], windows)}`, [local], [local], (operation) => host.request("operation.deny", { operationId: operation.operationId }));
    assert.equal(finished.type, "turn-completed", finished.message);
    assert.ok(!hasOutputLine(outputs.get(local.id) ?? "", `__DENIED_${mark}__`), "denied command ran");
    assert.ok(operationsIn(seen).some(({ status }) => status === "denied"), "no denied operation");
  });

  await step("an approved command reaches only its target", async () => {
    const mark = token();
    const other = await open({ kind: "shell" });
    const { finished, events: seen } = await turn(`Propose and then execute this exact command in terminal ${short}: ${printParts(["__LIVE_", `${mark}__`], windows)}`, [local], [local], async (operation) => {
      await host.request("operation.approve", { operationId: operation.operationId, scope: "once", mode: await localMode() });
    });
    assert.equal(finished.type, "turn-completed", finished.message);
    if (operationsIn(seen).at(-1).status === "staged") await manager.write({ id: local.id, data: "\r" });
    await until(() => hasOutputLine(outputs.get(local.id) ?? "", `__LIVE_${mark}__`), "the marker must appear", 20_000);
    assert.ok(!(outputs.get(other.id) ?? "").includes(mark), "output reached another terminal");
    await manager.close(other.id);
  });

  await step("cancelling stops the turn and voids its pending command", async () => {
    const start = events.length;
    const sending = turn(`Propose and then execute this exact command in terminal ${short}: ${windows ? "Start-Sleep -Seconds 120" : "sleep 120"}`, [local], [local]);
    try {
      const pending = await until(() => events.slice(start).find(({ type, operation }) => type === "operation" && operation.status === "pendingApproval"), "a proposal must arrive");
      await agent.cancel();
      assert.equal((await statusOf(pending.operation.operationId)).status, "cancelled");
    } finally { await sending; }
  });

  if (sshHost) {
    await step("SSH: a command is typed, never submitted, until Enter", async () => {
      const remote = await open({ kind: "ssh", profile: { host: sshHost }, companion: false });
      await delay(3000);
      const mark = token();
      const command = printParts(["__SSH_", `${mark}__`], false);
      let runRefused = false;
      const { finished } = await turn(`Propose and then execute this exact command in terminal ${remote.id.slice(0, 8)}: ${command}`, [remote], [remote], async (operation) => {
        await host.request("operation.approve", { operationId: operation.operationId, scope: "once", mode: "run" }).catch((error) => { runRefused = /never submitted/.test(error.message); });
        await host.request("operation.approve", { operationId: operation.operationId, scope: "once", mode: "stage" });
      });
      assert.equal(finished.type, "turn-completed", finished.message);
      assert.ok(runRefused, "SSH accepted a run approval");
      await delay(1000);
      assert.ok(!hasOutputLine(outputs.get(remote.id) ?? "", `__SSH_${mark}__`), "SSH command ran without Enter");
      await manager.write({ id: remote.id, data: "\r" });
      await until(() => hasOutputLine(outputs.get(remote.id) ?? "", `__SSH_${mark}__`), "Enter must run it", 20_000);
    });
  } else console.log("SKIP  SSH: set OMNITERM_LIVE_SSH_HOST");
  console.log(`\n${preset.label} / ${model} / ${profile.wireApi} / ${process.platform}: ${results.filter(Boolean).length}/${results.length} passed`);
  process.exitCode = results.every(Boolean) ? 0 : 1;
} finally {
  await agent.reset().catch(() => {});
  manager.dispose();
  rmSync(agentHome, { recursive: true, force: true, maxRetries: 5, retryDelay: 200 });
}
