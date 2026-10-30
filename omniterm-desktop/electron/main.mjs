// Copyright 2026 omniTerm contributors. SPDX-License-Identifier: Apache-2.0
import { app, BrowserWindow, dialog, ipcMain, Menu, nativeImage, shell } from "electron";
import * as nodePty from "node-pty";
import { randomUUID } from "node:crypto";
import { readFileSync, writeFileSync } from "node:fs";
import { copyFile, mkdir, readdir, rm, stat } from "node:fs/promises";
import path from "node:path";
import os from "node:os";
import { fileURLToPath, pathToFileURL } from "node:url";
import { buildCommandPlan, externalUrl, resolveDesktopAgent, validateProfile } from "./command-plan.mjs";
import { ProfileStore } from "./profile-store.mjs";
import { RemoteManager } from "./remote-manager.mjs";
import { SessionManager } from "./session-manager.mjs";
import { hostSummary, readSshHosts } from "./ssh-config.mjs";

let window;
let manager;
let remote;
let profiles;
let planOptions;
let rendererUrl;
let downloadDirectory;
let windowTransparent = false;

const IMAGE_EXTENSIONS = ["png", "jpg", "jpeg", "webp", "gif", "avif", "bmp"];
const MAX_IMAGE_BYTES = 32 * 1024 * 1024;
const preferencesFile = () => path.join(app.getPath("userData"), "window.json");
const backgroundDirectory = () => path.join(app.getPath("userData"), "backgrounds");

const hostProfile = (item) => ({ kind: "ssh", name: item.alias, host: item.alias, summary: hostSummary(item) });

// Aliases omniTerm could not pass to ssh safely are left out of the import list.
function importableHosts() {
  const result = readSshHosts();
  const valid = (item) => { try { return validateProfile(hostProfile(item), { requireConnection: true }); } catch { return null; } };
  return { ...result, hosts: result.hosts.filter(valid) };
}

// Read before the window exists: Electron fixes transparency at creation.
function readTransparency() {
  try { return JSON.parse(readFileSync(preferencesFile(), "utf8")).transparent === true; }
  catch { return false; }
}

// The image is copied so the background survives moving the original, and the
// renderer only ever receives a URL inside the app's own data directory.
async function chooseBackgroundImage() {
  const result = await dialog.showOpenDialog(window, { title: "Choose a background image", properties: ["openFile"], filters: [{ name: "Images", extensions: IMAGE_EXTENSIONS }] });
  const [source] = result.filePaths;
  if (result.canceled || !source) return null;
  const extension = path.extname(source).slice(1).toLowerCase();
  if (!IMAGE_EXTENSIONS.includes(extension)) throw new Error("请选择 PNG、JPEG、WebP、GIF、AVIF 或 BMP 图片。");
  if ((await stat(source)).size > MAX_IMAGE_BYTES) throw new Error("图片不能超过 32 MB。");
  const directory = backgroundDirectory();
  await mkdir(directory, { recursive: true });
  const name = `${randomUUID()}.${extension}`;
  await copyFile(source, path.join(directory, name));
  for (const file of await readdir(directory)) if (file !== name) await rm(path.join(directory, file), { force: true });
  return { url: pathToFileURL(path.join(directory, name)).href, name: path.basename(source) };
}

function send(channel, payload) {
  if (window && !window.isDestroyed() && !window.webContents.isDestroyed()) window.webContents.send(channel, payload);
}

function trustedSender(event) {
  if (!window || window.isDestroyed() || event.sender !== window.webContents || event.senderFrame !== window.webContents.mainFrame || event.senderFrame.url !== rendererUrl) throw new Error("IPC requests are only accepted from the omniTerm main window.");
}

// Remote first: it stops each SSH master synchronously through its control
// socket, which must still exist when the terminals behind them go away.
function disposeSessions() {
  remote?.dispose();
  manager?.dispose();
}

function registerHandlers() {
  const handlers = {
    "get-app-info": () => {
      let agent;
      try { const command = resolveDesktopAgent(planOptions); agent = { available: true, path: command.args[0] || command.executable }; }
      catch (error) { agent = { available: false, path: null, error: error.message }; }
      return { platform: process.platform, hostname: os.hostname(), home: os.homedir(), cwd: planOptions.defaultCwd, version: app.getVersion(), agent, profiles: profiles.list(), windowTransparent };
    },
    "choose-directory": async () => {
      const result = await dialog.showOpenDialog(window, { title: "Choose a working directory", defaultPath: planOptions.defaultCwd, properties: ["openDirectory", "createDirectory"] });
      return result.canceled ? null : result.filePaths[0] ?? null;
    },
    "create-session": (request) => manager.create(request),
    "start-session": (id) => manager.start(id),
    "write-session": (request) => manager.write(request),
    "resize-session": (request) => manager.resize(request),
    "close-session": (id) => manager.close(id),
    "get-session-buffer": (id) => manager.buffer(id),
    "save-profile": (profile) => profiles.save(profile),
    "delete-profile": (id) => profiles.delete(id),
    // Reads the file again on import, so only hosts that are really in it can be saved.
    "ssh-config-hosts": importableHosts,
    "import-ssh-hosts": (aliases) => {
      if (!Array.isArray(aliases)) throw new Error("Aliases must be a list.");
      return profiles.import(importableHosts().hosts.filter((item) => aliases.includes(item.alias)).map(hostProfile));
    },
    "choose-background-image": chooseBackgroundImage,
    "clear-background-image": () => rm(backgroundDirectory(), { recursive: true, force: true }),
    "set-window-transparency": (enabled) => { writeFileSync(preferencesFile(), JSON.stringify({ transparent: enabled === true })); return windowTransparent; },
    "relaunch": () => { app.relaunch(); app.quit(); },
    "open-external": (url) => shell.openExternal(externalUrl(url)),
    "remote-state": (id) => remote.state(id),
    "remote-retry": (id) => remote.retry(id),
    "remote-watch-metrics": (request) => remote.watchMetrics(request),
    "remote-list": (request) => remote.list(request),
    "remote-mkdir": (request) => remote.mkdir(request),
    "remote-rename": (request) => remote.rename(request),
    "remote-remove": (request) => remote.remove(request),
    "remote-upload": (request) => remote.upload(request),
    "remote-download": (request) => remote.download(request),
    "remote-cancel": (request) => remote.cancel(request),
    "remote-prepare-drag": (request) => remote.prepareDrag(request),
    "remote-choose-upload": async () => {
      const result = await dialog.showOpenDialog(window, { title: "Choose files to upload", properties: ["openFile", "multiSelections"] });
      return result.canceled ? [] : result.filePaths;
    },
    "remote-choose-download": async () => {
      const result = await dialog.showOpenDialog(window, { title: "Choose a download folder", defaultPath: downloadDirectory ?? app.getPath("downloads"), properties: ["openDirectory", "createDirectory"] });
      if (result.canceled) return null;
      [downloadDirectory] = result.filePaths;
      return downloadDirectory;
    },
  };
  for (const [name, handler] of Object.entries(handlers)) ipcMain.handle(`omniterm:${name}`, (event, payload) => { trustedSender(event); return handler(payload); });
  // startDrag must run while the renderer is still inside its dragstart event.
  ipcMain.on("omniterm:remote-start-drag", (event, request) => {
    try {
      trustedSender(event);
      const icon = nativeImage.createFromPath(path.join(app.getAppPath(), "assets", "icon.png")).resize({ width: 32, height: 32 });
      event.sender.startDrag({ file: remote.dragFile(request), icon });
    } catch (error) { send("omniterm:remote-drag-error", { message: error.message }); }
  });
}

function createWindow() {
  const appPath = app.getAppPath();
  const repositoryRoot = path.resolve(appPath, "..");
  planOptions = { platform: process.platform, env: process.env, repositoryRoot, packaged: app.isPackaged, resourcesPath: process.resourcesPath, defaultCwd: app.isPackaged ? os.homedir() : repositoryRoot };
  profiles = new ProfileStore(path.join(app.getPath("userData"), "profiles.json"));
  remote = new RemoteManager({ onState: (payload) => send("omniterm:remote-state", payload), onTransfer: (payload) => send("omniterm:remote-transfer", payload), onMetrics: (payload) => send("omniterm:remote-metrics", payload) });
  manager = new SessionManager({
    planner: (request) => buildCommandPlan(request, { ...planOptions, controlPath: () => remote.controlPath() }),
    spawnPty: (executable, args, options) => nodePty.spawn(executable, args, options),
    onData: (payload) => send("omniterm:session-data", payload),
    onStart: (id, plan) => { if (plan.remote) remote.attach(id, plan.remote); },
    onExit: (payload) => { remote.detach(payload.id); send("omniterm:session-exit", payload); },
    onClose: (id) => remote.detach(id),
  });
  const renderer = path.join(appPath, "build-renderer", "index.html");
  rendererUrl = pathToFileURL(renderer).href;
  windowTransparent = readTransparency();
  window = new BrowserWindow({ width: 1440, height: 940, minWidth: 1000, minHeight: 650, title: "omniTerm", ...(windowTransparent ? { transparent: true, backgroundColor: "#00000000" } : { backgroundColor: "#14181f" }), ...(process.platform === "darwin" ? { titleBarStyle: "hiddenInset", trafficLightPosition: { x: 20, y: 20 } } : {}), webPreferences: { preload: path.join(path.dirname(fileURLToPath(import.meta.url)), "preload.cjs"), contextIsolation: true, nodeIntegration: false, sandbox: true, webSecurity: true } });
  window.webContents.setWindowOpenHandler(() => ({ action: "deny" }));
  window.webContents.on("will-navigate", (event, url) => { if (url !== rendererUrl) event.preventDefault(); });
  window.webContents.on("will-redirect", (event) => event.preventDefault());
  window.webContents.on("will-attach-webview", (event) => event.preventDefault());
  const allowClipboard = (contents, permission, details) => contents === window?.webContents && details.isMainFrame === true && details.requestingUrl === rendererUrl && ["clipboard-read", "clipboard-sanitized-write"].includes(permission);
  window.webContents.session.setPermissionRequestHandler((contents, permission, callback, details) => callback(allowClipboard(contents, permission, details)));
  window.webContents.session.setPermissionCheckHandler((contents, permission, _origin, details) => allowClipboard(contents, permission, details));
  window.on("closed", () => { disposeSessions(); window = null; });
  window.webContents.on("render-process-gone", disposeSessions);
  window.loadFile(renderer);
}

app.setName("omniTerm");
app.whenReady().then(() => {
  Menu.setApplicationMenu(Menu.buildFromTemplate([
    ...(process.platform === "darwin" ? [{ role: "appMenu" }] : []),
    { role: "editMenu" },
    // Zoom accelerators are left out on purpose: Cmd/Ctrl +, - and 0 change the
    // terminal font size in the renderer instead of scaling the whole window.
    { label: "View", submenu: [{ role: "togglefullscreen" }] },
    { role: "windowMenu" },
  ]));
  registerHandlers();
  createWindow();
  app.on("activate", () => { if (!window) createWindow(); });
});
app.on("before-quit", disposeSessions);
app.on("window-all-closed", () => { if (process.platform !== "darwin") app.quit(); });
