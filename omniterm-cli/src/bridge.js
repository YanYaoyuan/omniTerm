// Copyright 2026 omniTerm contributors. SPDX-License-Identifier: Apache-2.0
import { readdir, readFile } from "node:fs/promises";
import net from "node:net";
import os from "node:os";
import path from "node:path";

export function discoveryDirectory(env = process.env, platform = process.platform) {
  if (env.OMNITERM_AUTOMATION_DIR) return env.OMNITERM_AUTOMATION_DIR;
  if (env.OMNITERM_DATA_DIR && path.isAbsolute(env.OMNITERM_DATA_DIR)) return path.join(env.OMNITERM_DATA_DIR, "automation");
  const home = env[platform === "win32" ? "USERPROFILE" : "HOME"] || os.homedir();
  const base = platform === "win32" ? (env.APPDATA || path.join(home, "AppData", "Roaming"))
    : platform === "darwin" ? path.join(home, "Library", "Application Support")
      : env.XDG_CONFIG_HOME && path.isAbsolute(env.XDG_CONFIG_HOME) ? env.XDG_CONFIG_HOME : path.join(home, ".config");
  return path.join(base, "omniTerm", "automation");
}

export function requestBridge(instance, method, params = {}, timeoutMs = 7000) {
  if (instance.protocol !== 1 || !Number.isInteger(instance.port) || instance.port < 1 || instance.port > 65535 || typeof instance.token !== "string") {
    return Promise.reject(new Error("Unsupported or invalid omniTerm discovery file"));
  }
  return new Promise((resolve, reject) => {
    const socket = net.createConnection({ host: "127.0.0.1", port: instance.port });
    let response = "";
    socket.setEncoding("utf8");
    socket.setTimeout(timeoutMs, () => socket.destroy(new Error("omniTerm request timed out; query the same request-id before retrying execution")));
    socket.on("connect", () => socket.write(JSON.stringify({ token: instance.token, method, params }) + "\n"));
    socket.on("error", reject);
    socket.on("data", (chunk) => {
      response += chunk;
      if (response.length > 8 * 1024 * 1024) { socket.destroy(new Error("omniTerm response exceeds 8 MiB")); return; }
      const end = response.indexOf("\n");
      if (end < 0) return;
      socket.end();
      try {
        const value = JSON.parse(response.slice(0, end));
        if (!value.ok) reject(new Error(value.error || "omniTerm request failed"));
        else resolve(value.result);
      } catch (error) { reject(error); }
    });
    socket.on("end", () => { if (!response.includes("\n")) reject(new Error("omniTerm disconnected; query the same request-id before retrying execution")); });
  });
}

export async function discoverInstances(directory) {
  let names;
  try { names = await readdir(directory); }
  catch (error) { if (error.code === "ENOENT") return []; throw error; }
  const candidates = names.filter((name) => name.endsWith(".json"));
  const found = await Promise.all(candidates.map(async (name) => {
    try {
      const instance = JSON.parse(await readFile(path.join(directory, name), "utf8"));
      const ping = await requestBridge(instance, "ping", {}, 500);
      if (ping.pid !== instance.pid) return null;
      return instance;
    } catch { return null; } // Stale files left by a crashed process are harmless.
  }));
  return found.filter(Boolean).sort((a, b) => a.instance.localeCompare(b.instance));
}

