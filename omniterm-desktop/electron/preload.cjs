// Copyright 2026 omniTerm contributors. SPDX-License-Identifier: Apache-2.0
const { contextBridge, ipcRenderer, webUtils } = require("electron");

function subscribe(channel, callback) {
  if (typeof callback !== "function") throw new TypeError("A callback is required.");
  const listener = (_event, payload) => callback(payload);
  ipcRenderer.on(channel, listener);
  return () => ipcRenderer.removeListener(channel, listener);
}

// Electron prefixes every rejected call with its channel name, which means nothing to a user.
function invoke(channel, payload) {
  return ipcRenderer.invoke(channel, payload).catch((error) => {
    throw new Error(String(error?.message ?? error).replace(/^Error invoking remote method '[^']*': (?:\w*Error: )?/, ""));
  });
}

contextBridge.exposeInMainWorld("omniterm", Object.freeze({
  getAppInfo: () => invoke("omniterm:get-app-info"),
  chooseDirectory: () => invoke("omniterm:choose-directory"),
  createSession: (request) => invoke("omniterm:create-session", request),
  startSession: (id) => invoke("omniterm:start-session", id),
  writeSession: (request) => invoke("omniterm:write-session", request),
  resizeSession: (request) => invoke("omniterm:resize-session", request),
  closeSession: (id) => invoke("omniterm:close-session", id),
  getSessionBuffer: (id) => invoke("omniterm:get-session-buffer", id),
  stageShellIntegration: (id) => invoke("omniterm:stage-shell-integration", id),
  allowAutoSubmit: (request) => invoke("omniterm:allow-auto-submit", request),
  openProcessWindow: (host) => invoke("omniterm:open-process-window", host),
  processSnapshot: (host) => invoke("omniterm:process-snapshot", host),
  startProcessMonitor: (request) => invoke("omniterm:process-monitor-start", request),
  monitorList: () => invoke("omniterm:monitor-list"),
  monitorCreate: (request) => invoke("omniterm:monitor-create", request),
  monitorApprove: (id) => invoke("omniterm:monitor-approve", id),
  monitorDeny: (id) => invoke("omniterm:monitor-deny", id),
  monitorPause: (id) => invoke("omniterm:monitor-pause", id),
  monitorResume: (id) => invoke("omniterm:monitor-resume", id),
  monitorStop: (id) => invoke("omniterm:monitor-stop", id),
  monitorRevokeAction: (id) => invoke("omniterm:monitor-revoke-action", id),
  monitorRemove: (id) => invoke("omniterm:monitor-remove", id),
  updateWorkspace: (layout) => invoke("omniterm:workspace-update", layout),
  listWorkspace: () => invoke("omniterm:workspace-list"),
  providerState: () => invoke("omniterm:provider-state"),
  saveProvider: (request) => invoke("omniterm:provider-save", request),
  deleteProvider: (id) => invoke("omniterm:provider-delete", id),
  testProvider: (id) => invoke("omniterm:provider-test", id),
  agentSend: (request) => invoke("omniterm:agent-send", request),
  agentCancel: () => invoke("omniterm:agent-cancel"),
  agentPause: (request) => invoke("omniterm:agent-pause", request),
  agentReset: () => invoke("omniterm:agent-reset"),
  approveOperation: (request) => invoke("omniterm:operation-approve", request),
  denyOperation: (id) => invoke("omniterm:operation-deny", id),
  cancelOperation: (id) => invoke("omniterm:operation-cancel", id),
  saveProfile: (profile) => invoke("omniterm:save-profile", profile),
  deleteProfile: (id) => invoke("omniterm:delete-profile", id),
  sshConfigHosts: () => invoke("omniterm:ssh-config-hosts"),
  importSshHosts: (aliases) => invoke("omniterm:import-ssh-hosts", aliases),
  chooseBackgroundImage: () => invoke("omniterm:choose-background-image"),
  clearBackgroundImage: () => invoke("omniterm:clear-background-image"),
  setWindowTransparency: (enabled) => invoke("omniterm:set-window-transparency", enabled),
  relaunch: () => invoke("omniterm:relaunch"),
  openExternal: (url) => invoke("omniterm:open-external", url),
  remoteState: (id) => invoke("omniterm:remote-state", id),
  remoteRetry: (id) => invoke("omniterm:remote-retry", id),
  remoteWatchMetrics: (request) => invoke("omniterm:remote-watch-metrics", request),
  remoteList: (request) => invoke("omniterm:remote-list", request),
  remoteMkdir: (request) => invoke("omniterm:remote-mkdir", request),
  remoteRename: (request) => invoke("omniterm:remote-rename", request),
  remoteRemove: (request) => invoke("omniterm:remote-remove", request),
  remoteUpload: (request) => invoke("omniterm:remote-upload", request),
  remoteDownload: (request) => invoke("omniterm:remote-download", request),
  remoteCancel: (request) => invoke("omniterm:remote-cancel", request),
  remotePrepareDrag: (request) => invoke("omniterm:remote-prepare-drag", request),
  remoteStartDrag: (request) => ipcRenderer.send("omniterm:remote-start-drag", request),
  remoteChooseUpload: () => invoke("omniterm:remote-choose-upload"),
  remoteChooseDownload: () => invoke("omniterm:remote-choose-download"),
  // Electron no longer exposes File.path; resolve dropped files explicitly.
  pathForFile: (file) => webUtils.getPathForFile(file),
  onSessionData: (callback) => subscribe("omniterm:session-data", callback),
  onSessionExit: (callback) => subscribe("omniterm:session-exit", callback),
  onAgentEvent: (callback) => subscribe("omniterm:agent-event", callback),
  onMonitorEvent: (callback) => subscribe("omniterm:monitor-event", callback),
  onRemoteState: (callback) => subscribe("omniterm:remote-state", callback),
  onRemoteTransfer: (callback) => subscribe("omniterm:remote-transfer", callback),
  onRemoteMetrics: (callback) => subscribe("omniterm:remote-metrics", callback),
  onRemoteDragError: (callback) => subscribe("omniterm:remote-drag-error", callback),
}));
