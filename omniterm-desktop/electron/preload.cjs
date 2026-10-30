// Copyright 2026 omniTerm contributors. SPDX-License-Identifier: Apache-2.0
const { contextBridge, ipcRenderer, webUtils } = require("electron");

function subscribe(channel, callback) {
  if (typeof callback !== "function") throw new TypeError("A callback is required.");
  const listener = (_event, payload) => callback(payload);
  ipcRenderer.on(channel, listener);
  return () => ipcRenderer.removeListener(channel, listener);
}

contextBridge.exposeInMainWorld("omniterm", Object.freeze({
  getAppInfo: () => ipcRenderer.invoke("omniterm:get-app-info"),
  chooseDirectory: () => ipcRenderer.invoke("omniterm:choose-directory"),
  createSession: (request) => ipcRenderer.invoke("omniterm:create-session", request),
  startSession: (id) => ipcRenderer.invoke("omniterm:start-session", id),
  writeSession: (request) => ipcRenderer.invoke("omniterm:write-session", request),
  resizeSession: (request) => ipcRenderer.invoke("omniterm:resize-session", request),
  closeSession: (id) => ipcRenderer.invoke("omniterm:close-session", id),
  getSessionBuffer: (id) => ipcRenderer.invoke("omniterm:get-session-buffer", id),
  saveProfile: (profile) => ipcRenderer.invoke("omniterm:save-profile", profile),
  deleteProfile: (id) => ipcRenderer.invoke("omniterm:delete-profile", id),
  openExternal: (url) => ipcRenderer.invoke("omniterm:open-external", url),
  // Electron no longer exposes File.path; resolve dropped files explicitly.
  pathForFile: (file) => webUtils.getPathForFile(file),
  onSessionData: (callback) => subscribe("omniterm:session-data", callback),
  onSessionExit: (callback) => subscribe("omniterm:session-exit", callback),
}));
