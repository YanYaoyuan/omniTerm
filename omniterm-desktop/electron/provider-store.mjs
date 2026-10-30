// Copyright 2026 omniTerm contributors. SPDX-License-Identifier: Apache-2.0
import { randomUUID } from "node:crypto";
import { mkdirSync, readFileSync, renameSync, writeFileSync } from "node:fs";
import path from "node:path";

// Base URLs are starting points; regions and gateways differ, so each stays editable.
export const PROVIDER_PRESETS = Object.freeze({
  openai: { label: "OpenAI", wireApi: "responses", baseUrl: "https://api.openai.com/v1", protocols: ["responses"] },
  "openai-login": { label: "OpenAI（现有登录）", wireApi: "responses", baseUrl: "", protocols: ["responses"], auth: "engine-login" },
  qwen: { label: "Qwen / 阿里云百炼", wireApi: "chat", baseUrl: "https://dashscope.aliyuncs.com/compatible-mode/v1", protocols: ["chat", "responses"] },
  anthropic: { label: "Claude / Anthropic", wireApi: "anthropic_messages", baseUrl: "https://api.anthropic.com/v1", protocols: ["anthropic_messages"] },
  kimi: { label: "Kimi / Moonshot", wireApi: "chat", baseUrl: "https://api.moonshot.ai/v1", protocols: ["chat"] },
  openrouter: { label: "OpenRouter", wireApi: "chat", baseUrl: "https://openrouter.ai/api/v1", protocols: ["chat", "responses"] },
  custom: { label: "自定义端点", wireApi: "chat", baseUrl: "", protocols: ["chat", "responses", "anthropic_messages"] },
});
const WIRE_APIS = new Set(["responses", "chat", "anthropic_messages"]);
const MAX_PROFILES = 50;
const CONTROL = /[\u0000-\u001f\u007f]/;

function text(value, label, maximum) {
  if (typeof value !== "string" || !value.trim() || value.length > maximum || CONTROL.test(value)) throw new Error(`${label} must be text of at most ${maximum} characters.`);
  return value.trim();
}

export function validateBaseUrl(value) {
  let url;
  try { url = new URL(text(value, "Base URL", 2048)); } catch { throw new Error("Base URL is not a valid URL."); }
  const loopback = ["localhost", "127.0.0.1", "[::1]"].includes(url.hostname);
  if (url.protocol !== "https:" && !(url.protocol === "http:" && loopback)) throw new Error("Base URL must use https (http is only allowed for localhost).");
  if (url.username || url.password || url.search || url.hash) throw new Error("Base URL cannot contain credentials or a query string.");
  return url.href.replace(/\/$/, "");
}

export function validateProviderProfile(input = {}) {
  const preset = PROVIDER_PRESETS[input.presetId];
  if (!Object.hasOwn(PROVIDER_PRESETS, input.presetId ?? "")) throw new Error("Unknown provider preset.");
  if (!preset.protocols.includes(input.wireApi) || !WIRE_APIS.has(input.wireApi)) throw new Error(`${preset.label} does not implement that protocol.`);
  if (input.id !== undefined && !/^[a-zA-Z0-9_-]{1,80}$/.test(input.id)) throw new Error("Invalid profile id.");
  const budget = input.contextBudgetBytes ?? 64 * 1024;
  if (!Number.isSafeInteger(budget) || budget < 4096 || budget > 512 * 1024) throw new Error("The context budget must be 4–512 KiB.");
  const auth = preset.auth ?? "api-key";
  return {
    id: input.id || randomUUID(), presetId: input.presetId, name: text(input.name, "Name", 120), auth, wireApi: input.wireApi,
    baseUrl: auth === "engine-login" ? "" : validateBaseUrl(input.baseUrl),
    model: text(input.model, "Model ID", 200),
    chatOnly: input.chatOnly === true,
    contextBudgetBytes: budget,
    verified: input.verified && typeof input.verified === "object" ? { connection: input.verified.connection === true, tools: input.verified.tools === true, at: Number(input.verified.at) || null } : { connection: false, tools: false, at: null },
  };
}

function writeAtomic(filename, content, mode) {
  mkdirSync(path.dirname(filename), { recursive: true });
  const temporary = `${filename}.${process.pid}.tmp`;
  writeFileSync(temporary, content, { mode });
  renameSync(temporary, filename);
}

export class ProviderStore {
  constructor(filename) { this.filename = filename; }
  list() {
    try {
      const value = JSON.parse(readFileSync(this.filename, "utf8"));
      return Array.isArray(value.profiles) ? value.profiles.flatMap((profile) => { try { return [validateProviderProfile(profile)]; } catch { return []; } }) : [];
    } catch (error) {
      if (error.code === "ENOENT") return [];
      throw new Error("The model settings file is unreadable.");
    }
  }
  write(profiles) { writeAtomic(this.filename, `${JSON.stringify({ version: 1, profiles }, null, 2)}\n`, 0o600); return profiles; }
  get(id) {
    const profile = this.list().find((item) => item.id === id);
    if (!profile) throw new Error("Choose a model configuration first.");
    return profile;
  }
  save(input) {
    const profile = validateProviderProfile(input);
    const profiles = this.list();
    const index = profiles.findIndex((item) => item.id === profile.id);
    // Editing what a profile connects to voids what was verified about it.
    const changed = index < 0 || ["wireApi", "baseUrl", "model"].some((key) => profiles[index][key] !== profile[key]);
    if (changed) profile.verified = { connection: false, tools: false, at: null };
    if (index >= 0) profiles[index] = profile;
    else if (profiles.length >= MAX_PROFILES) throw new Error(`A maximum of ${MAX_PROFILES} model configurations can be saved.`);
    else profiles.push(profile);
    this.write(profiles);
    return profile;
  }
  delete(id) { return this.write(this.list().filter((item) => item.id !== id)); }
  recordVerification(id, verified) {
    const profiles = this.list();
    const profile = profiles.find((item) => item.id === id);
    if (profile) { profile.verified = { ...verified, at: Date.now() }; this.write(profiles); }
    return profiles;
  }
}

// API keys stay out of settings, logs and the renderer. Without an OS-backed store they live in
// memory for this run: a key is never written in a form that anyone with the file can read.
export class CredentialVault {
  constructor({ filename, safeStorage, platform = process.platform }) {
    this.filename = filename;
    this.safeStorage = safeStorage;
    this.platform = platform;
    this.session = new Map();
  }
  get persistent() {
    if (!this.safeStorage?.isEncryptionAvailable()) return false;
    // Electron's Linux fallback uses a hard-coded password when no keyring is running.
    return this.platform !== "linux" || this.safeStorage.getSelectedStorageBackend() !== "basic_text";
  }
  readFile() {
    try { return JSON.parse(readFileSync(this.filename, "utf8")); }
    catch { return {}; }
  }
  set(id, secret) {
    const value = text(secret, "API key", 8192);
    if (!this.persistent) { this.session.set(id, value); return { persistent: false }; }
    const store = this.readFile();
    store[id] = this.safeStorage.encryptString(value).toString("base64");
    writeAtomic(this.filename, JSON.stringify(store), 0o600);
    this.session.delete(id);
    return { persistent: true };
  }
  get(id) {
    if (this.session.has(id)) return this.session.get(id);
    if (!this.persistent) return null;
    const encrypted = this.readFile()[id];
    if (!encrypted) return null;
    try { return this.safeStorage.decryptString(Buffer.from(encrypted, "base64")); }
    catch { return null; }
  }
  has(id) { return this.session.has(id) || (this.persistent && Boolean(this.readFile()[id])); }
  delete(id) {
    this.session.delete(id);
    const store = this.readFile();
    if (id in store) { delete store[id]; writeAtomic(this.filename, JSON.stringify(store), 0o600); }
  }
}
