// Copyright 2026 omniTerm contributors. SPDX-License-Identifier: Apache-2.0
import { execFile } from "node:child_process";

// The script travels encoded, so nothing in it passes through a command line parser.
export function localProbeCommand(script, platform = process.platform) {
  if (platform !== "win32") return ["sh", ["-c", script]];
  return ["powershell.exe", ["-NoProfile", "-NonInteractive", "-EncodedCommand", Buffer.from(script, "utf16le").toString("base64")]];
}

export function runLocalProbe(script, { platform = process.platform, timeoutMs = 15_000 } = {}) {
  const [file, args] = localProbeCommand(script, platform);
  return new Promise((resolve, reject) => {
    execFile(file, args, { timeout: timeoutMs, maxBuffer: 2 * 1024 * 1024, windowsHide: true, encoding: "utf8" }, (error, stdout) => {
      if (error) reject(new Error("本机采样失败。")); else resolve(stdout);
    });
  });
}
