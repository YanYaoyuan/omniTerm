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
  stageShellIntegration: (id) => ipcRenderer.invoke("omniterm:stage-shell-integration", id),
  updateWorkspace: (layout) => ipcRenderer.invoke("omniterm:workspace-update", layout),
  listWorkspace: () => ipcRenderer.invoke("omniterm:workspace-list"),
  providerState: () => ipcRenderer.invoke("omniterm:provider-state"),
  saveProvider: (request) => ipcRenderer.invoke("omniterm:provider-save", request),
  deleteProvider: (id) => ipcRenderer.invoke("omniterm:provider-delete", id),
  testProvider: (id) => ipcRenderer.invoke("omniterm:provider-test", id),
  agentSend: (request) => ipcRenderer.invoke("omniterm:agent-send", request),
  agentCancel: () => ipcRenderer.invoke("omniterm:agent-cancel"),
  agentPause: (request) => ipcRenderer.invoke("omniterm:agent-pause", request),
  agentReset: () => ipcRenderer.invoke("omniterm:agent-reset"),
  approveOperation: (request) => ipcRenderer.invoke("omniterm:operation-approve", request),
  denyOperation: (id) => ipcRenderer.invoke("omniterm:operation-deny", id),
  cancelOperation: (id) => ipcRenderer.invoke("omniterm:operation-cancel", id),
  saveProfile: (profile) => ipcRenderer.invoke("omniterm:save-profile", profile),
  deleteProfile: (id) => ipcRenderer.invoke("omniterm:delete-profile", id),
  sshConfigHosts: () => ipcRenderer.invoke("omniterm:ssh-config-hosts"),
  importSshHosts: (aliases) => ipcRenderer.invoke("omniterm:import-ssh-hosts", aliases),
  chooseBackgroundImage: () => ipcRenderer.invoke("omniterm:choose-background-image"),
  clearBackgroundImage: () => ipcRenderer.invoke("omniterm:clear-background-image"),
  setWindowTransparency: (enabled) => ipcRenderer.invoke("omniterm:set-window-transparency", enabled),
  relaunch: () => ipcRenderer.invoke("omniterm:relaunch"),
  openExternal: (url) => ipcRenderer.invoke("omniterm:open-external", url),
  remoteState: (id) => ipcRenderer.invoke("omniterm:remote-state", id),
  remoteRetry: (id) => ipcRenderer.invoke("omniterm:remote-retry", id),
  remoteWatchMetrics: (request) => ipcRenderer.invoke("omniterm:remote-watch-metrics", request),
  remoteList: (request) => ipcRenderer.invoke("omniterm:remote-list", request),
  remoteMkdir: (request) => ipcRenderer.invoke("omniterm:remote-mkdir", request),
  remoteRename: (request) => ipcRenderer.invoke("omniterm:remote-rename", request),
  remoteRemove: (request) => ipcRenderer.invoke("omniterm:remote-remove", request),
  remoteUpload: (request) => ipcRenderer.invoke("omniterm:remote-upload", request),
  remoteDownload: (request) => ipcRenderer.invoke("omniterm:remote-download", request),
  remoteCancel: (request) => ipcRenderer.invoke("omniterm:remote-cancel", request),
  remotePrepareDrag: (request) => ipcRenderer.invoke("omniterm:remote-prepare-drag", request),
  remoteStartDrag: (request) => ipcRenderer.send("omniterm:remote-start-drag", request),
  remoteChooseUpload: () => ipcRenderer.invoke("omniterm:remote-choose-upload"),
  remoteChooseDownload: () => ipcRenderer.invoke("omniterm:remote-choose-download"),
  // Electron no longer exposes File.path; resolve dropped files explicitly.
  pathForFile: (file) => webUtils.getPathForFile(file),
  onSessionData: (callback) => subscribe("omniterm:session-data", callback),
  onSessionExit: (callback) => subscribe("omniterm:session-exit", callback),
  onAgentEvent: (callback) => subscribe("omniterm:agent-event", callback),
  onRemoteState: (callback) => subscribe("omniterm:remote-state", callback),
  onRemoteTransfer: (callback) => subscribe("omniterm:remote-transfer", callback),
  onRemoteMetrics: (callback) => subscribe("omniterm:remote-metrics", callback),
  onRemoteDragError: (callback) => subscribe("omniterm:remote-drag-error", callback),
}));
