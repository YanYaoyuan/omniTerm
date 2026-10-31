// Copyright 2026 omniTerm contributors. SPDX-License-Identifier: Apache-2.0
import { spawn } from "node:child_process";
import { createInterface } from "node:readline";
import { AGENT_INSTRUCTIONS, TERMINAL_TOOLS } from "./agent-tools.mjs";

const REQUEST_TIMEOUT_MS = 60_000;
export const ENGINE_PROVIDER_ID = "omniterm";
// Seen only if the engine offered a built-in tool: terminal mode must stop, not continue.
const FORBIDDEN_ITEMS = new Set(["commandExecution", "fileChange", "mcpToolCall", "webSearch", "collabAgentToolCall", "imageGeneration"]);

const tomlString = (value) => JSON.stringify(value);

// Everything is passed per launch, so a user's engine config file is never rewritten.
export function engineArguments(profile) {
  const overrides = {
    "features.dynamic_tools_only": "true",
    "features.code_mode": "false",
    "features.hooks": "false",
    "analytics.enabled": "false",
    "history.persistence": tomlString("none"),
    model: tomlString(profile.model),
  };
  if (profile.auth !== "engine-login") {
    Object.assign(overrides, {
      model_provider: tomlString(ENGINE_PROVIDER_ID),
      [`model_providers.${ENGINE_PROVIDER_ID}.name`]: tomlString(profile.name),
      [`model_providers.${ENGINE_PROVIDER_ID}.base_url`]: tomlString(profile.baseUrl),
      [`model_providers.${ENGINE_PROVIDER_ID}.wire_api`]: tomlString(profile.wireApi),
      [`model_providers.${ENGINE_PROVIDER_ID}.env_key`]: tomlString(CREDENTIAL_ENV),
      [`model_providers.${ENGINE_PROVIDER_ID}.requires_openai_auth`]: "false",
    });
  }
  return Object.entries(overrides).flatMap(([key, value]) => ["-c", `${key}=${value}`]);
}

export const CREDENTIAL_ENV = "OMNITERM_PROVIDER_API_KEY";

// One engine process per provider profile: the key reaches that process alone, and a turn
// can never fall back to another provider.
export class AgentRuntime {
  constructor({ command, profile, credential, env, codexHome, spawnProcess = spawn, onNotification = () => {}, onToolCall, onExit = () => {} }) {
    this.command = command;
    this.profile = profile;
    this.credential = credential;
    this.env = env;
    this.codexHome = codexHome;
    this.spawnProcess = spawnProcess;
    this.onNotification = onNotification;
    this.onToolCall = onToolCall;
    this.onExit = onExit;
    this.pending = new Map();
    this.nextId = 1;
    this.child = null;
    this.stopped = false;
  }
  async start({ clientVersion }) {
    const env = { ...this.env };
    delete env[CREDENTIAL_ENV];
    if (this.credential) env[CREDENTIAL_ENV] = this.credential;
    if (this.codexHome) env.CODEX_HOME = this.codexHome;
    const args = [...this.command.args, ...engineArguments(this.profile), "app-server", "--listen", "stdio://"];
    const child = this.spawnProcess(this.command.executable, args, { env, stdio: ["pipe", "pipe", "ignore"], windowsHide: true });
    this.child = child;
    createInterface({ input: child.stdout }).on("line", (line) => this.receive(line));
    child.stdin.on("error", () => { /* Reported through the exit event. */ });
    const failed = new Promise((_, reject) => {
      child.once("error", (error) => reject(new Error(`The Agent engine could not start: ${error.message}`)));
      child.once("exit", (code) => {
        const error = new Error(`The Agent engine stopped${code === null ? "" : ` (exit code ${code})`}.`);
        for (const { reject: fail, timer } of this.pending.values()) { clearTimeout(timer); fail(error); }
        this.pending.clear();
        reject(error);
        if (!this.stopped) this.onExit(error);
      });
    });
    failed.catch(() => {});
    await Promise.race([failed, this.request("initialize", { clientInfo: { name: "omniterm_desktop", title: "omniTerm", version: clientVersion }, capabilities: { experimentalApi: true } })]);
    this.send({ method: "initialized" });
  }
  send(message) { this.child?.stdin.write(`${JSON.stringify(message)}\n`); }
  request(method, params, timeoutMs = REQUEST_TIMEOUT_MS) {
    return new Promise((resolve, reject) => {
      if (!this.child) { reject(new Error("The Agent engine is not running.")); return; }
      const id = this.nextId++;
      const timer = setTimeout(() => { this.pending.delete(id); reject(new Error(`The Agent engine did not answer ${method}.`)); }, timeoutMs);
      this.pending.set(id, { resolve, reject, timer });
      this.send({ id, method, params });
    });
  }
  receive(line) {
    let message;
    try { message = JSON.parse(line); } catch { return; }
    if (message.method && message.id !== undefined) { this.serve(message); return; }
    if (message.method) { this.onNotification(message.method, message.params); return; }
    const request = this.pending.get(message.id);
    if (!request) return;
    this.pending.delete(message.id);
    clearTimeout(request.timer);
    if (message.error) request.reject(new Error(message.error.message));
    else request.resolve(message.result);
  }
  // Requests the engine makes of its client. Only client tools are served; anything else is a
  // built-in path that terminal mode has disabled, so it is refused.
  async serve({ id, method, params }) {
    if (method === "item/tool/call") {
      let result;
      try {
        const output = await this.onToolCall(params);
        result = { contentItems: [{ type: "inputText", text: typeof output === "string" ? output : JSON.stringify(output) }], success: true };
      } catch (error) {
        result = { contentItems: [{ type: "inputText", text: error.message || String(error) }], success: false };
      }
      this.send({ id, result });
      return;
    }
    const declined = { "item/commandExecution/requestApproval": { decision: "decline" }, "item/fileChange/requestApproval": { decision: "decline" }, "item/tool/requestUserInput": { answers: {} } }[method];
    if (declined) this.send({ id, result: declined });
    else this.send({ id, error: { code: -32601, message: `omniTerm does not serve ${method}.` } });
  }
  async startThread({ cwd, tools }) {
    const result = await this.request("thread/start", {
      model: this.profile.model,
      cwd,
      approvalPolicy: "never",
      sandbox: "read-only",
      ephemeral: true,
      serviceName: "omniterm_desktop",
      developerInstructions: AGENT_INSTRUCTIONS,
      environments: [],
      dynamicTools: tools ? TERMINAL_TOOLS : [],
    });
    return result.thread.id;
  }
  async startTurn(threadId, text) {
    const result = await this.request("turn/start", { threadId, input: [{ type: "text", text, text_elements: [] }] });
    return result.turn.id;
  }
  interrupt(threadId, turnId) { return this.request("turn/interrupt", { threadId, turnId }); }
  stop() {
    this.stopped = true;
    const child = this.child;
    this.child = null;
    if (child) { child.stdin.end(); child.kill(); }
  }
}

export const isForbiddenItem = (item) => FORBIDDEN_ITEMS.has(item?.type);

// Model output is rendered by the UI; these are the events it needs.
export function normalizeNotification(method, params) {
  switch (method) {
    case "turn/started": return { type: "turn-started", turnId: params.turn?.id };
    case "item/agentMessage/delta": return { type: "text-delta", itemId: params.itemId, delta: params.delta };
    case "item/reasoning/textDelta":
    case "item/reasoning/summaryTextDelta": return { type: "reasoning-delta", itemId: params.itemId, delta: params.delta };
    case "item/started":
    case "item/completed": {
      const item = params.item;
      if (item?.type === "agentMessage") return method === "item/completed" ? { type: "text-done", itemId: item.id, text: item.text } : null;
      if (item?.type === "dynamicToolCall") return { type: "tool", itemId: item.id, tool: item.tool, status: item.status, success: item.success ?? null };
      return null;
    }
    case "turn/completed": return { type: "turn-completed", turnId: params.turn?.id, status: params.turn?.status, error: params.turn?.error?.message ?? null };
    case "error": return { type: params.willRetry ? "retrying" : "error", message: params.error?.message ?? "The model request failed." };
    default: return null;
  }
}

// Provider failures are the same across engines; the message says what to do about them.
export function actionableError(message = "") {
  const text = message.toLowerCase();
  if (/\b401\b|\b403\b|unauthorized|invalid[_ ]api[_ ]key|authentication|rejected the api key/.test(text)) return `服务拒绝了凭据，请在模型设置中检查 API Key 和权限。（${message}）`;
  if (/\b429\b|rate.?limit|overloaded|quota|usage limit/.test(text)) return `请求受到限速或额度不足，请稍后重试或检查服务额度。（${message}）`;
  if (/timed? ?out|idle timeout|stopped responding/.test(text)) return `模型服务响应超时，未执行后续操作。请检查网络和 Base URL。（${message}）`;
  if (/context.*(window|length)|too many tokens|prompt is too long/.test(text)) return `内容超出模型上下文，请减少共享终端或新建对话。（${message}）`;
  if (/\b404\b|model.*not.*found|does not exist/.test(text)) return `找不到该模型或端点，请核对模型 ID、协议和 Base URL。（${message}）`;
  if (/tool|function/.test(text) && /support|invalid|unavailable/.test(text)) return `该模型可能不支持工具调用，可在模型设置中改为“仅聊天”。（${message}）`;
  return message;
}
