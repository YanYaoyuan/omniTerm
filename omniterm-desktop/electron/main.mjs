// Copyright 2026 omniTerm contributors. SPDX-License-Identifier: Apache-2.0
import { app, BrowserWindow, dialog, ipcMain, Menu, shell } from "electron";
import * as nodePty from "node-pty";
import path from "node:path";
import os from "node:os";
import { fileURLToPath, pathToFileURL } from "node:url";
import { buildCommandPlan, externalUrl, resolveDesktopAgent } from "./command-plan.mjs";
import { ProfileStore } from "./profile-store.mjs";
import { SessionManager } from "./session-manager.mjs";

let window;
let manager;
let profiles;
let planOptions;
let rendererUrl;

function send(channel, payload) {
  if (window && !window.isDestroyed() && !window.webContents.isDestroyed()) window.webContents.send(channel, payload);
}

function trustedSender(event) {
  if (!window || window.isDestroyed() || event.sender !== window.webContents || event.senderFrame !== window.webContents.mainFrame || event.senderFrame.url !== rendererUrl) throw new Error("IPC requests are only accepted from the omniTerm main window.");
}

function registerHandlers() {
  const handlers = {
    "get-app-info": () => {
      let agent;
      try { const command = resolveDesktopAgent(planOptions); agent = { available: true, path: command.args[0] || command.executable }; }
      catch (error) { agent = { available: false, path: null, error: error.message }; }
      return { platform: process.platform, home: os.homedir(), cwd: planOptions.defaultCwd, version: app.getVersion(), agent, profiles: profiles.list() };
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
    "open-external": (url) => shell.openExternal(externalUrl(url)),
  };
  for (const [name, handler] of Object.entries(handlers)) ipcMain.handle(`omniterm:${name}`, (event, payload) => { trustedSender(event); return handler(payload); });
}

function createWindow() {
  const appPath = app.getAppPath();
  const repositoryRoot = path.resolve(appPath, "..");
  planOptions = { platform: process.platform, env: process.env, repositoryRoot, packaged: app.isPackaged, resourcesPath: process.resourcesPath, defaultCwd: app.isPackaged ? os.homedir() : repositoryRoot };
  profiles = new ProfileStore(path.join(app.getPath("userData"), "profiles.json"));
  manager = new SessionManager({ planner: (request) => buildCommandPlan(request, planOptions), spawnPty: (executable, args, options) => nodePty.spawn(executable, args, options), onData: (payload) => send("omniterm:session-data", payload), onExit: (payload) => send("omniterm:session-exit", payload) });
  const renderer = path.join(appPath, "build-renderer", "index.html");
  rendererUrl = pathToFileURL(renderer).href;
  window = new BrowserWindow({ width: 1440, height: 940, minWidth: 1000, minHeight: 650, title: "omniTerm", backgroundColor: "#14181f", ...(process.platform === "darwin" ? { titleBarStyle: "hiddenInset", trafficLightPosition: { x: 20, y: 20 } } : {}), webPreferences: { preload: path.join(path.dirname(fileURLToPath(import.meta.url)), "preload.cjs"), contextIsolation: true, nodeIntegration: false, sandbox: true, webSecurity: true } });
  window.webContents.setWindowOpenHandler(() => ({ action: "deny" }));
  window.webContents.on("will-navigate", (event, url) => { if (url !== rendererUrl) event.preventDefault(); });
  window.webContents.on("will-redirect", (event) => event.preventDefault());
  window.webContents.on("will-attach-webview", (event) => event.preventDefault());
  const allowClipboard = (contents, permission, details) => contents === window?.webContents && details.isMainFrame === true && details.requestingUrl === rendererUrl && ["clipboard-read", "clipboard-sanitized-write"].includes(permission);
  window.webContents.session.setPermissionRequestHandler((contents, permission, callback, details) => callback(allowClipboard(contents, permission, details)));
  window.webContents.session.setPermissionCheckHandler((contents, permission, _origin, details) => allowClipboard(contents, permission, details));
  window.on("closed", () => { manager?.dispose(); window = null; });
  window.webContents.on("render-process-gone", () => manager?.dispose());
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
app.on("before-quit", () => manager?.dispose());
app.on("window-all-closed", () => { if (process.platform !== "darwin") app.quit(); });
