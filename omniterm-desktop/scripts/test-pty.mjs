import assert from "node:assert/strict";
import { spawn } from "node:child_process";
import path from "node:path";
import { fileURLToPath } from "node:url";

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
  let output = "";
  const manager = new SessionManager({
    planner: (request) => buildCommandPlan(request, { repositoryRoot, defaultCwd: repositoryRoot, env }),
    spawnPty: (...args) => pty.spawn(...args),
    onData: ({ data }) => { output += data; },
  });
  const delay = (ms) => new Promise((resolve) => setTimeout(resolve, ms));
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
    write(windows ? "Write-Output '__OMNITERM_PTY_READY__'; (Get-Location).Path\r" : "printf '__OMNITERM_PTY_READY__\\n'; pwd\r");
    await waitFor(() => output.includes("\r\n__OMNITERM_PTY_READY__\r\n") && output.includes(repositoryRoot), "The shell must execute input and report its working directory");
    manager.resize({ id: session.id, cols: 112, rows: 37 });
    if (!windows) {
      write("stty size\r");
      await waitFor(() => output.includes("37 112"), "PTY resize must reach the shell");
    }
    write(windows ? "Start-Sleep -Seconds 30\r" : "sleep 30\r");
    await delay(300);
    write("\x03");
    await delay(200);
    write(windows ? "Write-Output '__OMNITERM_INTERRUPT_OK__'\r" : "printf '__OMNITERM_INTERRUPT_OK__\\n'\r");
    await waitFor(() => output.includes("\r\n__OMNITERM_INTERRUPT_OK__\r\n"), "Ctrl+C must return to a usable shell");
    manager.close(session.id);
    if (!windows) {
      await waitFor(() => { try { process.kill(running.pid, 0); return false; } catch (error) { return error.code === "ESRCH"; } }, "Closing a session must stop its shell");
    }
    console.log("PASS: Electron PTY input, working directory, resize, Ctrl+C, and process cleanup");
  } finally {
    manager.dispose();
  }
}
