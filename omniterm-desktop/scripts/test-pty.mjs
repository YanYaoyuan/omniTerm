import assert from "node:assert/strict";
import { spawn } from "node:child_process";
import path from "node:path";
import { fileURLToPath } from "node:url";
import { stripVTControlCharacters } from "node:util";

const here = fileURLToPath(import.meta.url);
if (!process.versions.electron) {
  const { default: electron } = await import("electron");
  const child = spawn(electron, [here], {
    env: { ...process.env, ELECTRON_RUN_AS_NODE: "1" },
    stdio: "inherit",
  });
  child.once("error", (error) => { console.error(error); process.exitCode = 1; });
  child.once("exit", (code) => { process.exitCode = code ?? 1; });
} else {
  const pty = await import("node-pty");
  const { SessionManager } = await import("../electron/session-manager.mjs");
  const { buildCommandPlan } = await import("../electron/command-plan.mjs");
  const repositoryRoot = path.resolve(path.dirname(here), "../..");
  const env = { ...process.env };
  delete env.ELECTRON_RUN_AS_NODE;
  // Exercise the character-locale fallback used when launched by a desktop
  // session or CI worker with no UTF-8 locale configured.
  if (process.platform === "linux") { env.LANG = "C"; env.LC_ALL = "C"; }
  let output = "";
  const manager = new SessionManager({
    planner: (request) => buildCommandPlan(request, { repositoryRoot, defaultCwd: repositoryRoot, env }),
    spawnPty: (...args) => pty.spawn(...args),
    onData: ({ data }) => { output += data; },
  });
  const delay = (ms) => new Promise((resolve) => setTimeout(resolve, ms));
  // Readline can emit bracketed-paste controls between the echoed command and
  // its output. Compare complete rendered lines, never just a marker substring.
  const hasOutputLine = (line) => stripVTControlCharacters(output).replaceAll("\r", "").split("\n").includes(line);
  const printParts = (parts, windows) => windows
    ? `Write-Output (${parts.map((part) => `'${part}'`).join(" + ")})`
    : `printf '${"%s".repeat(parts.length)}\\n' ${parts.map((part) => `'${part}'`).join(" ")}`;
  const waitFor = async (predicate, label) => {
    const deadline = Date.now() + 15000;
    while (!predicate() && Date.now() < deadline) await delay(50);
    assert.ok(predicate(), `${label}\n${output.slice(-2500)}`);
  };
  try {
    const session = await manager.create({ kind: "shell" });
    const running = manager.start(session.id);
    const write = (data) => manager.write({ id: session.id, data });
    const windows = process.platform === "win32";
    // Each marker is assembled by the shell; echoing the command cannot satisfy
    // the assertion even if the terminal wraps or inserts control sequences.
    write(`${printParts(["__OMNITERM_", "PTY_READY__"], windows)}; ${windows ? "(Get-Location).Path" : "pwd"}\r`);
    await waitFor(() => hasOutputLine("__OMNITERM_PTY_READY__") && hasOutputLine(repositoryRoot), "The shell must execute input and report its working directory");
    write(`${printParts(["__OMNITERM_中文", "输入_OK__"], windows)}\r`);
    await waitFor(() => hasOutputLine("__OMNITERM_中文输入_OK__"), "The shell must preserve Chinese input as UTF-8");
    if (!windows) {
      write(`${printParts(["__OMNITERM_EDIT_中文错\x7f", "输入_OK__"], false)}\r`);
      await waitFor(() => hasOutputLine("__OMNITERM_EDIT_中文输入_OK__"), "Backspace must remove an entire Chinese character without corrupting UTF-8");
    }
    manager.resize({ id: session.id, cols: 112, rows: 37 });
    if (!windows) {
      write("stty size\r");
      await waitFor(() => hasOutputLine("37 112"), "PTY resize must reach the shell");
    }
    write(windows ? "Start-Sleep -Seconds 30\r" : "sleep 30\r");
    await delay(300);
    write("\x03");
    await delay(200);
    write(`${printParts(["__OMNITERM_", "INTERRUPT_OK__"], windows)}\r`);
    await waitFor(() => hasOutputLine("__OMNITERM_INTERRUPT_OK__"), "Ctrl+C must return to a usable shell");
    manager.close(session.id);
    if (!windows) {
      await waitFor(() => { try { process.kill(running.pid, 0); return false; } catch (error) { return error.code === "ESRCH"; } }, "Closing a session must stop its shell");
    }
    console.log("PASS: Electron PTY input, UTF-8 Chinese input, working directory, resize, Ctrl+C, and process cleanup");
  } finally {
    manager.dispose();
  }
}
