// Drives the real Electron window over the DevTools protocol, with no extra dependencies.
// Drops use synthetic drag events; dividers use real mouse input. Needs a display (or xvfb-run),
// a build (npm run build) and a terminal host. Screenshots go to OMNITERM_GUI_ARTIFACTS.
import assert from "node:assert/strict";
import { spawn } from "node:child_process";
import { mkdirSync, mkdtempSync, rmSync, writeFileSync } from "node:fs";
import os from "node:os";
import path from "node:path";
import { fileURLToPath } from "node:url";
import electron from "electron";

const root = fileURLToPath(new URL("../", import.meta.url));
const artifacts = process.env.OMNITERM_GUI_ARTIFACTS || mkdtempSync(path.join(os.tmpdir(), "omniterm-gui-"));
mkdirSync(artifacts, { recursive: true });
const profile = mkdtempSync(path.join(os.tmpdir(), "omniterm-profile-"));
const delay = (ms) => new Promise((resolve) => setTimeout(resolve, ms));
const until = async (predicate, label, timeoutMs = 30_000) => {
  const deadline = Date.now() + timeoutMs;
  for (;;) {
    const value = await predicate().catch((error) => { if (error.fatal) throw error; return null; });
    if (value) return value;
    if (Date.now() > deadline) throw new Error(`Timed out: ${label}`);
    await delay(100);
  }
};

async function launch() {
  const port = 9300 + Math.floor(Math.random() * 500);
  const env = { ...process.env };
  delete env.ELECTRON_RUN_AS_NODE;
  const child = spawn(electron, [root, `--remote-debugging-port=${port}`, `--user-data-dir=${profile}`, "--no-sandbox", "--disable-gpu"], { env, stdio: ["ignore", "ignore", "pipe"] });
  let stderr = "";
  child.stderr.on("data", (chunk) => { stderr = (stderr + chunk).slice(-2000); });
  const target = await until(async () => {
    if (child.exitCode !== null) throw Object.assign(new Error(`Electron exited with ${child.exitCode}: ${stderr}`), { fatal: true });
    const pages = await (await fetch(`http://127.0.0.1:${port}/json/list`)).json();
    return pages.find(({ type, url }) => type === "page" && url.includes("index.html"));
  }, "the window");
  const socket = new WebSocket(target.webSocketDebuggerUrl);
  await new Promise((resolve, reject) => { socket.onopen = resolve; socket.onerror = reject; });
  const pending = new Map();
  let nextId = 1;
  socket.onmessage = ({ data }) => {
    const message = JSON.parse(data);
    const request = pending.get(message.id);
    if (!request) return;
    pending.delete(message.id);
    if (message.error) request.reject(new Error(message.error.message)); else request.resolve(message.result);
  };
  const send = (method, params = {}) => new Promise((resolve, reject) => {
    const id = nextId++;
    pending.set(id, { resolve, reject });
    socket.send(JSON.stringify({ id, method, params }));
  });
  const run = async (expression) => {
    const { result, exceptionDetails } = await send("Runtime.evaluate", { expression, awaitPromise: true, returnByValue: true });
    if (exceptionDetails) throw new Error(exceptionDetails.exception?.description ?? exceptionDetails.text);
    return result.value;
  };
  const close = async () => {
    socket.close();
    child.kill();
    await new Promise((resolve) => { child.once("exit", resolve); setTimeout(resolve, 5000); });
  };
  return { send, run, close };
}

const client = (app) => {
  const rect = (selector) => app.run(`(() => { const box = document.querySelector(${JSON.stringify(selector)}).getBoundingClientRect(); return { left: box.left, top: box.top, right: box.right, bottom: box.bottom, width: box.width, height: box.height }; })()`);
  const click = (selector) => app.run(`document.querySelector(${JSON.stringify(selector)}).click()`);
  const drag = (source, target, x, y) => app.run(`(async () => {
    const pause = () => new Promise((resolve) => setTimeout(resolve, 120));
    const from = document.querySelector(${JSON.stringify(source)});
    const dataTransfer = new DataTransfer();
    from.dispatchEvent(new DragEvent("dragstart", { bubbles: true, dataTransfer }));
    await pause();
    const to = document.querySelector(${JSON.stringify(target)});
    const box = to.getBoundingClientRect();
    const init = { bubbles: true, cancelable: true, dataTransfer, clientX: box.left + box.width * ${x}, clientY: box.top + box.height * ${y} };
    to.dispatchEvent(new DragEvent("dragover", init));
    await pause();
    to.dispatchEvent(new DragEvent("drop", init));
    from.dispatchEvent(new DragEvent("dragend", { bubbles: true, dataTransfer }));
    await pause();
  })()`);
  const screenshot = async (name) => {
    const { data } = await app.send("Page.captureScreenshot", { format: "png" });
    writeFileSync(path.join(artifacts, `${name}.png`), Buffer.from(data, "base64"));
  };
  const terminals = async () => (await app.run("window.omniterm.listWorkspace()")).terminals;
  const shellPid = async (id, label) => {
    await app.run(`window.omniterm.writeSession({ id: ${JSON.stringify(id)}, data: "echo PID_$$_${label}\\r" })`);
    return until(async () => {
      const { data } = await app.run(`window.omniterm.getSessionBuffer(${JSON.stringify(id)})`);
      return new RegExp(`PID_(\\d+)_${label}`).exec(data.replaceAll("$$", ""))?.[1];
    }, `the shell must answer ${label}`);
  };
  return { ...app, rect, click, drag, screenshot, terminals, shellPid };
};

const step = (name) => console.log(`PASS  ${name}`);
const panels = async (app) => Object.fromEntries(await Promise.all(Object.entries({ agent: ".agent-pane", sidebar: ".sidebar", terminals: ".main-workspace" }).map(async ([name, selector]) => [name, await app.rect(selector)])));

let app = client(await launch());
try {
  await until(async () => (await app.terminals()).length > 0, "the first terminal");
  await until(() => app.run(`Boolean(document.querySelector(".terminal-view.is-visible .xterm"))`), "the terminal view");
  const [first] = await app.terminals();
  const id = first.ref.sessionId;
  const pid = await app.shellPid(id, "START");
  await app.run(`[...document.querySelectorAll("[aria-label^='向右分屏']")][0].click()`);
  await until(async () => (await app.terminals()).length === 2, "the split pane");
  const second = (await app.terminals()).find(({ ref }) => ref.sessionId !== id).ref.sessionId;
  const secondPid = await app.shellPid(second, "SPLIT");

  await app.click(".agent-scope .scope-chip");
  await until(() => app.run(`Boolean(document.querySelector(".terminal-picker"))`), "the terminal picker");
  await app.run(`document.querySelector(".terminal-picker input[type=radio]:not(:disabled)").click()`);
  const scope = () => app.run(`({ chips: [...document.querySelectorAll(".scope-chip")].map((chip) => chip.textContent), target: [...document.querySelectorAll(".terminal-picker input[type=radio]")].map((input) => input.checked) })`);
  await until(async () => (await scope()).target.includes(true), "the execution target");
  const selected = await scope();
  await app.screenshot("1-default-layout");

  const before = await panels(app);
  assert.ok(before.sidebar.left < before.terminals.left && before.terminals.left < before.agent.left, "default order");

  await app.drag(".agent-heading .panel-grip", ".pane-drop-target[data-panel=sidebar]", 0.05, 0.5);
  const agentMoved = await panels(app);
  assert.ok(agentMoved.agent.right <= agentMoved.sidebar.left + 8, "the Agent panel docks left of the sidebar");
  step("the Agent panel moves");

  await app.drag(".sidebar-grip .panel-grip", ".pane-drop-target[data-panel=terminals]", 0.5, 0.05);
  const sidebarMoved = await panels(app);
  assert.ok(sidebarMoved.sidebar.bottom <= sidebarMoved.terminals.top + 8, "the sidebar docks above the terminals");
  step("the workspace panel moves");
  await app.screenshot("2-panels-moved");

  const grip = ".terminal-view.is-visible.is-focused .pane-grip, .terminal-view.is-visible .pane-grip";
  const panesBefore = await app.run(`[...document.querySelectorAll(".terminal-view.is-visible")].map((view) => view.getBoundingClientRect().left)`);
  await app.drag(grip, ".pane-drop-target", 0.5, 0.95);
  const panesAfter = await app.run(`[...document.querySelectorAll(".terminal-view.is-visible")].map((view) => view.getBoundingClientRect().left)`);
  assert.equal(panesAfter.length, 2);
  assert.notDeepEqual(panesAfter, panesBefore, "the panes must stack instead of sitting side by side");
  step("a terminal pane moves");

  const divider = await app.rect(".dock-divider.row");
  // Away from the middle, where this divider meets the horizontal one.
  const [x, y] = [divider.left + divider.width / 2, divider.top + divider.height / 4];
  const widthBefore = (await panels(app)).agent.width;
  await app.send("Input.dispatchMouseEvent", { type: "mouseMoved", x, y });
  await app.send("Input.dispatchMouseEvent", { type: "mousePressed", x, y, button: "left", buttons: 1, clickCount: 1 });
  await app.send("Input.dispatchMouseEvent", { type: "mouseMoved", x: x + 45, y, button: "left", buttons: 1 });
  await app.send("Input.dispatchMouseEvent", { type: "mouseMoved", x: x + 90, y, button: "left", buttons: 1 });
  await app.send("Input.dispatchMouseEvent", { type: "mouseReleased", x: x + 90, y, button: "left", buttons: 0, clickCount: 1 });
  await delay(300);
  const resized = await panels(app);
  assert.ok(Math.abs(resized.agent.width - widthBefore) > 40, "the docked region resizes");
  step("docked regions resize");
  await app.screenshot("3-resized");

  assert.equal(await app.shellPid(id, "AFTER"), pid, "the first terminal is the same process");
  assert.equal(await app.shellPid(second, "AFTERTWO"), secondPid, "the second terminal is the same process");
  const { data } = await app.run(`window.omniterm.getSessionBuffer(${JSON.stringify(id)})`);
  assert.ok(data.includes("PID_"), "earlier output is still there");
  step("running sessions and their output survive every move");

  assert.deepEqual(await scope(), selected);
  step("Agent panel state survives every move (open picker, context, execution target)");

  await app.close();
  app = client(await launch());
  await until(async () => (await app.terminals()).length > 0, "the restored terminals");
  await until(() => app.run(`Boolean(document.querySelector(".agent-pane.is-visible"))`), "the Agent panel");
  await delay(600);
  const restored = await panels(app);
  for (const name of ["agent", "sidebar", "terminals"]) for (const edge of ["left", "top", "width", "height"]) assert.ok(Math.abs(restored[name][edge] - resized[name][edge]) < 3, `${name} ${edge}: ${restored[name][edge]} vs ${resized[name][edge]}`);
  step("the arrangement and sizes are restored after reopening");
  await app.screenshot("4-reopened");
  console.log(`\nPASS: dockable workspace. Screenshots in ${artifacts}`);
} finally {
  await app.close().catch(() => {});
  rmSync(profile, { recursive: true, force: true, maxRetries: 5, retryDelay: 200 });
}
