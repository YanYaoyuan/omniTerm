import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import {
  ArrowDown, ArrowRight, ArrowUp, ArrowsLeftRight, ArrowsOut, ArrowsIn, Broadcast, CaretRight, Check, Circle,
  CircleNotch, Code, Command, Copy, Cpu, DotsThree, FolderOpen, FolderSimple,
  GearSix, HardDrives, Info, MagnifyingGlass, Monitor, PaperPlaneTilt, Palette, PencilSimple,
  PlugsConnected, Plus, Robot, SidebarSimple, SquareSplitHorizontal, SquareSplitVertical, TerminalWindow, Trash, X,
  ClipboardText, WarningCircle, SlidersHorizontal, ArrowClockwise, TextAa,
} from "@phosphor-icons/react";
import { TerminalView } from "./TerminalView.jsx";
import { ConnectionDialog, connectionLabels } from "./ConnectionDialog.jsx";
import { CommandPalette } from "./CommandPalette.jsx";
import { computeLayout, equalize, neighborPane, pane, paneIds, removePane, setRatio, splitPane } from "./layout.js";
import { SHORTCUTS, matchShortcut, shortcutLabel, tabCycle, tabNumber } from "./shortcuts.js";
import { THEMES, themeFor } from "./themes.js";
import { WORKSPACE_KEY, parseWorkspace, restoreTree, serializeWorkspace } from "./workspace.js";

const api = window.omniterm;
export const defaults = {
  fontSize: 13, fontFamily: "system", lineHeight: 1.22, scrollback: 10000, cursorBlink: true, cursorStyle: "bar",
  theme: "midnight", gpu: true, copyOnSelect: false, optionAsMeta: false, dimInactive: true, restoreWorkspace: true,
};
const kindIcons = { shell: TerminalWindow, agent: Robot, ssh: HardDrives, sftp: FolderOpen, serial: PlugsConnected, tunnel: ArrowsLeftRight };
const platformNames = { darwin: "macOS", win32: "Windows", linux: "Linux" };
const fontNames = { system: "系统等宽字体", jetbrains: "JetBrains Mono", fira: "Fira Code", cascadia: "Cascadia Code", sarasa: "Sarasa Mono（中英等宽）" };
const basename = (value = "") => value.split(/[/\\]/).filter(Boolean).at(-1) || value;
const pathLabel = (value, home) => value && home && value.startsWith(home) ? `~${value.slice(home.length)}` : value;
const finished = (status) => ["exited", "error"].includes(status);
const DIVIDER = 6;
let tabCounter = 0;
const newTabId = () => `tab-${Date.now().toString(36)}-${++tabCounter}`;

function readSettings() {
  try { return { ...defaults, ...JSON.parse(localStorage.getItem("omniterm-terminal-settings") || "{}") }; }
  catch { return defaults; }
}

// Converts a fractional rectangle to absolute CSS with a gutter between panes.
function paneStyle(rect) {
  const edge = (start, size) => ({ start: start > 0 ? DIVIDER / 2 : 0, end: start + size < 1 - 1e-6 ? DIVIDER / 2 : 0 });
  const horizontal = edge(rect.x, rect.w);
  const vertical = edge(rect.y, rect.h);
  return {
    left: `calc(${rect.x * 100}% + ${horizontal.start}px)`,
    top: `calc(${rect.y * 100}% + ${vertical.start}px)`,
    width: `calc(${rect.w * 100}% - ${horizontal.start + horizontal.end}px)`,
    height: `calc(${rect.h * 100}% - ${vertical.start + vertical.end}px)`,
    right: "auto", bottom: "auto",
  };
}

function IconButton({ label, children, className = "", ...props }) {
  return <button type="button" className={`icon-button ${className}`} title={label} aria-label={label} {...props}>{children}</button>;
}

function Toggle({ value, label, onChange }) {
  return <button className={`switch ${value ? "on" : ""}`} role="switch" aria-checked={value} aria-label={label} onClick={() => onChange(!value)}><span /></button>;
}

function SettingsDialog({ settings, setSettings, info, onClose }) {
  const dialog = useRef(null);
  useEffect(() => { dialog.current.showModal(); }, []);
  const update = (key) => (value) => setSettings((previous) => ({ ...previous, [key]: value }));
  const theme = themeFor(settings.theme);
  return <dialog className="modal settings-modal" ref={dialog} onCancel={(event) => { event.preventDefault(); onClose(); }} onClick={(event) => { if (event.target === dialog.current) onClose(); }}>
    <div className="modal-heading"><div className="modal-heading-icon"><SlidersHorizontal size={22} /></div><div><h2>终端设置</h2><p>让工作区更适合你的习惯</p></div><IconButton className="modal-close" label="关闭设置" onClick={onClose}><X size={18} /></IconButton></div>
    <div className="settings-content">
      <div className="settings-section-label">外观</div>
      <div className="theme-grid" role="radiogroup" aria-label="配色方案">
        {Object.entries(THEMES).map(([id, item]) => <button key={id} role="radio" aria-checked={settings.theme === id} className={`theme-swatch ${settings.theme === id ? "active" : ""}`} onClick={() => update("theme")(id)} style={{ background: item.terminal.background, color: item.terminal.foreground }}>
          <span className="theme-colors">{["red", "green", "yellow", "blue", "magenta", "cyan"].map((color) => <i key={color} style={{ background: item.terminal[color] }} />)}</span>
          <span>{item.name}</span>
        </button>)}
      </div>
      <div className="setting-row"><div><strong>字体</strong><span>未安装的字体会回退到系统等宽字体</span></div><select value={settings.fontFamily} onChange={(event) => update("fontFamily")(event.target.value)}>{Object.entries(fontNames).map(([id, name]) => <option key={id} value={id}>{name}</option>)}</select></div>
      <div className="setting-row"><div><strong>字体大小</strong><span>应用到所有终端和 Agent 会话</span></div><div className="number-stepper"><button onClick={() => update("fontSize")(Math.max(9, settings.fontSize - 1))} disabled={settings.fontSize <= 9} aria-label="缩小字体">−</button><span>{settings.fontSize} px</span><button onClick={() => update("fontSize")(Math.min(28, settings.fontSize + 1))} disabled={settings.fontSize >= 28} aria-label="放大字体">+</button></div></div>
      <div className="setting-row"><div><strong>行高</strong><span>行与行之间的距离</span></div><select value={settings.lineHeight} onChange={(event) => update("lineHeight")(Number(event.target.value))}>{[1, 1.1, 1.22, 1.35, 1.5].map((value) => <option key={value} value={value}>{value}</option>)}</select></div>
      <div className="font-preview" style={{ fontSize: settings.fontSize, lineHeight: settings.lineHeight, background: theme.terminal.background, color: theme.terminal.foreground }}>omniTerm ~/workspace $ <span style={{ color: theme.terminal.green }}>git</span> status<span className="sample-cursor" style={{ background: theme.terminal.cursor }} /><br /><span style={{ color: theme.terminal.blue }}>中文对齐</span> → <span style={{ color: theme.terminal.yellow }}>✓ 0 errors</span></div>
      <div className="setting-row"><div><strong>光标样式</strong><span>输入位置的光标形状</span></div><select value={settings.cursorStyle} onChange={(event) => update("cursorStyle")(event.target.value)}><option value="bar">竖线</option><option value="block">方块</option><option value="underline">下划线</option></select></div>
      <div className="setting-row"><div><strong>光标闪烁</strong><span>在输入位置显示闪烁光标</span></div><Toggle value={settings.cursorBlink} label="光标闪烁" onChange={update("cursorBlink")} /></div>
      <div className="setting-row"><div><strong>淡化非活动窗格</strong><span>分屏时突出当前焦点窗格</span></div><Toggle value={settings.dimInactive} label="淡化非活动窗格" onChange={update("dimInactive")} /></div>
      <div className="settings-section-label">行为与性能</div>
      <div className="setting-row"><div><strong>GPU 加速渲染</strong><span>使用 WebGL 渲染终端，大量输出时更流畅</span></div><Toggle value={settings.gpu} label="GPU 加速渲染" onChange={update("gpu")} /></div>
      <div className="setting-row"><div><strong>选中即复制</strong><span>选择文本后自动写入剪贴板</span></div><Toggle value={settings.copyOnSelect} label="选中即复制" onChange={update("copyOnSelect")} /></div>
      {info.platform === "darwin" && <div className="setting-row"><div><strong>Option 作为 Meta 键</strong><span>用于 Emacs、readline 的 Alt 组合键</span></div><Toggle value={settings.optionAsMeta} label="Option 作为 Meta 键" onChange={update("optionAsMeta")} /></div>}
      <div className="setting-row"><div><strong>启动时恢复工作区</strong><span>重新打开本地标签、分屏布局和各窗格目录，不会重新执行命令</span></div><Toggle value={settings.restoreWorkspace} label="启动时恢复工作区" onChange={update("restoreWorkspace")} /></div>
      <div className="setting-row"><div><strong>回滚缓冲区</strong><span>保留每个终端的历史输出行数</span></div><select value={settings.scrollback} onChange={(event) => update("scrollback")(Number(event.target.value))}><option value="1000">1,000 行</option><option value="5000">5,000 行</option><option value="10000">10,000 行</option><option value="50000">50,000 行</option><option value="100000">100,000 行</option></select></div>
      <div className="settings-section-label">键盘快捷键</div>
      {Object.entries(SHORTCUTS).map(([name, definition]) => <div className="shortcut-row" key={name}><span>{definition.label}</span><kbd>{shortcutLabel(name, info.platform)}</kbd></div>)}
      <div className="shortcut-row"><span>切换标签</span><kbd>Ctrl+Tab</kbd></div>
      <div className="shortcut-row"><span>跳转到第 N 个标签</span><kbd>{info.platform === "darwin" ? "⌘1…9" : "Alt+1…9"}</kbd></div>
      <div className="shortcut-row"><span>打开链接</span><kbd>{info.platform === "darwin" ? "⌘" : "Ctrl"}+点击</kbd></div>
      <div className="settings-about"><div className="mini-logo"><TerminalWindow size={20} /></div><div><strong>omniTerm <span>{info.version}</span></strong><p>你的终端，你的 Agent。</p></div><span className="platform-pill">{platformNames[info.platform] || info.platform}</span></div>
    </div>
    <div className="modal-actions"><button className="button ghost" onClick={() => setSettings(defaults)}>恢复默认</button><button className="button primary" onClick={onClose}>完成<Check size={15} /></button></div>
  </dialog>;
}

function DeleteDialog({ profile, onClose, onConfirm }) {
  const dialog = useRef(null);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState("");
  useEffect(() => { dialog.current.showModal(); }, []);
  return <dialog ref={dialog} className="modal delete-modal" onCancel={(event) => { event.preventDefault(); if (!busy) onClose(); }}>
    <div className="modal-heading"><div className="modal-heading-icon"><Trash size={22} /></div><div><h2>删除保存的连接？</h2><p>{profile.name}</p></div></div>
    <p className="delete-explanation">连接配置将从此设备移除，已经打开的会话不受影响。</p>
    {error && <div className="inline-error">{error}</div>}
    <div className="modal-actions"><span /><div className="action-group"><button className="button ghost" disabled={busy} onClick={onClose}>取消</button><button className="button danger" disabled={busy} onClick={async () => { setBusy(true); try { await onConfirm(profile.id); onClose(); } catch (failure) { setError(failure.message); setBusy(false); } }}>删除连接</button></div></div>
  </dialog>;
}

function TabTitle({ tab, title, onRename }) {
  const [editing, setEditing] = useState(false);
  const [value, setValue] = useState(title);
  if (!editing) return <span onDoubleClick={(event) => { event.stopPropagation(); setValue(title); setEditing(true); }} title="双击重命名">{title}</span>;
  const commit = () => { setEditing(false); onRename(tab.id, value.trim()); };
  return <input className="tab-rename" autoFocus value={value} maxLength={60} onChange={(event) => setValue(event.target.value)} onBlur={commit} onClick={(event) => event.stopPropagation()} onKeyDown={(event) => { event.stopPropagation(); if (event.key === "Enter") commit(); if (event.key === "Escape") setEditing(false); }} />;
}

export function App() {
  const [info, setInfo] = useState(null);
  const [bootError, setBootError] = useState("");
  const [cwd, setCwd] = useState("");
  const [sessions, setSessions] = useState([]);
  const [tabs, setTabs] = useState([]);
  const [activeTabId, setActiveTabId] = useState(null);
  const [focusedId, setFocusedId] = useState(null);
  const [agentSession, setAgentSession] = useState(null);
  const [agentVisible, setAgentVisible] = useState(true);
  const [agentStarting, setAgentStarting] = useState(false);
  const [profiles, setProfiles] = useState([]);
  const [profileMenu, setProfileMenu] = useState(null);
  const [dialog, setDialog] = useState(null);
  const [settingsOpen, setSettingsOpen] = useState(false);
  const [paletteOpen, setPaletteOpen] = useState(false);
  const [deleting, setDeleting] = useState(null);
  const [settings, setSettings] = useState(readSettings);
  const [toast, setToast] = useState(null);
  const [searchOpen, setSearchOpen] = useState(false);
  const [searchValue, setSearchValue] = useState("");
  const [searchFound, setSearchFound] = useState(true);
  const [searchOptions, setSearchOptions] = useState({ caseSensitive: false, regex: false });
  const [draggedTab, setDraggedTab] = useState(null);
  const controls = useRef(new Map());
  const toastTimer = useRef(null);
  const searchInput = useRef(null);
  const stackRef = useRef(null);
  const started = useRef(false);
  const ready = useRef(false);
  const nextTerminal = useRef(1);
  const tabsRef = useRef(tabs);
  const sessionsRef = useRef(sessions);
  tabsRef.current = tabs;
  sessionsRef.current = sessions;

  const notify = useCallback((message, type = "error") => {
    clearTimeout(toastTimer.current);
    setToast({ message, type });
    toastTimer.current = setTimeout(() => setToast(null), 6500);
  }, []);
  const register = useCallback((id, value) => {
    if (value) controls.current.set(id, value); else controls.current.delete(id);
  }, []);
  const patchSession = useCallback((id, patch) => {
    setSessions((previous) => previous.map((session) => session.id === id ? { ...session, ...patch } : session));
    setAgentSession((previous) => previous?.id === id ? { ...previous, ...patch } : previous);
  }, []);
  const onState = useCallback((id, status, exitCode) => patchSession(id, { status, exitCode }), [patchSession]);
  const onCwd = useCallback((id, liveCwd) => patchSession(id, { liveCwd }), [patchSession]);
  const onTitle = useCallback((id, termTitle) => patchSession(id, { termTitle }), [patchSession]);
  const focusLater = useCallback((id) => requestAnimationFrame(() => controls.current.get(id)?.focus()), []);
  const tabOf = useCallback((id) => tabsRef.current.find((tab) => paneIds(tab.root).includes(id)), []);

  const focusPane = useCallback((id) => {
    setFocusedId(id);
    setTabs((previous) => {
      const index = previous.findIndex((tab) => paneIds(tab.root).includes(id));
      if (index < 0 || previous[index].focus === id) return previous;
      const next = [...previous];
      next[index] = { ...next[index], focus: id };
      return next;
    });
  }, []);

  const activatePane = useCallback((id) => {
    const tab = tabOf(id);
    if (tab) setActiveTabId(tab.id);
    focusPane(id);
    focusLater(id);
  }, [tabOf, focusPane, focusLater]);

  const startSession = useCallback(async (kind = "shell", profile, directory) => {
    const session = await api.createSession({ kind, cwd: directory || cwd, ...(profile ? { profile } : {}) });
    const title = profile?.name || (kind === "shell" ? `本地终端 ${nextTerminal.current++}` : connectionLabels[kind] || session.title);
    const entry = { ...session, title, ...(profile ? { profile } : {}) };
    setSessions((previous) => [...previous, entry]);
    return entry;
  }, [cwd]);

  const addTab = useCallback(async (kind = "shell", profile, directory) => {
    const session = await startSession(kind, profile, directory);
    const tab = { id: newTabId(), root: pane(session.id), focus: session.id, title: "", broadcast: false, zoomed: null };
    setTabs((previous) => [...previous, tab]);
    setActiveTabId(tab.id);
    setFocusedId(session.id);
    return session;
  }, [startSession]);

  const splitFocused = useCallback(async (direction) => {
    const tab = tabsRef.current.find((item) => item.id === activeTabId);
    if (!tab) { await addTab(); return; }
    const source = sessionsRef.current.find((session) => session.id === tab.focus);
    // Remote sessions duplicate their connection, like iTerm's "split with
    // current profile". Serial devices and tunnels cannot be opened twice.
    const duplicate = source && ["ssh", "sftp"].includes(source.kind) && source.profile;
    const directory = source?.liveCwd || source?.cwd;
    const session = await startSession(duplicate ? source.kind : "shell", duplicate ? source.profile : undefined, source?.kind === "shell" ? directory : undefined);
    setTabs((previous) => previous.map((item) => item.id === tab.id ? { ...item, root: splitPane(item.root, item.focus, session.id, direction), focus: session.id, zoomed: null } : item));
    setFocusedId(session.id);
  }, [activeTabId, addTab, startSession]);

  // Removes a tab whose sessions are already closed and moves focus to a neighbor.
  const dropTab = useCallback((tabId) => {
    const index = tabsRef.current.findIndex((item) => item.id === tabId);
    const remaining = tabsRef.current.filter((item) => item.id !== tabId);
    setTabs((previous) => previous.filter((item) => item.id !== tabId));
    if (activeTabId === tabId) {
      const next = remaining[Math.min(index, remaining.length - 1)];
      setActiveTabId(next?.id || null);
      setFocusedId(next?.focus || null);
      if (next) focusLater(next.focus);
    }
  }, [activeTabId, focusLater]);

  const closePane = useCallback(async (id) => {
    try {
      await api.closeSession(id);
      if (agentSession?.id === id) {
        setAgentSession(null);
        const fallback = tabsRef.current.find((tab) => tab.id === activeTabId)?.focus;
        if (fallback) activatePane(fallback);
        return;
      }
      const tab = tabOf(id);
      setSessions((previous) => previous.filter((session) => session.id !== id));
      if (!tab) return;
      const root = removePane(tab.root, id);
      if (!root) { dropTab(tab.id); return; }
      const focus = tab.focus === id ? neighborPane(tab.root, id, "left") || neighborPane(tab.root, id, "up") || paneIds(root)[0] : tab.focus;
      setTabs((previous) => previous.map((item) => {
        if (item.id !== tab.id) return item;
        const nextRoot = removePane(item.root, id);
        return { ...item, root: nextRoot, focus: paneIds(nextRoot).includes(focus) ? focus : paneIds(nextRoot)[0], zoomed: item.zoomed === id ? null : item.zoomed };
      }));
      if (focusedId === id) { setFocusedId(focus); focusLater(focus); }
    } catch (error) { notify(error.message); }
  }, [agentSession, activeTabId, focusedId, tabOf, activatePane, dropTab, focusLater, notify]);

  const closeTab = useCallback(async (tabId) => {
    const tab = tabsRef.current.find((item) => item.id === tabId);
    if (!tab) return;
    const ids = paneIds(tab.root);
    const results = await Promise.allSettled(ids.map((id) => api.closeSession(id)));
    const failure = results.find((result) => result.status === "rejected");
    if (failure) notify(failure.reason?.message || String(failure.reason));
    setSessions((previous) => previous.filter((session) => !ids.includes(session.id)));
    dropTab(tabId);
  }, [dropTab, notify]);

  const onInput = useCallback((id, data) => {
    const tab = tabOf(id);
    const targets = tab?.broadcast ? paneIds(tab.root).filter((target) => target === id || !controls.current.get(target)?.isExited?.()) : [id];
    for (const target of targets) Promise.resolve(api.writeSession({ id: target, data })).catch((error) => notify(error.message));
  }, [tabOf, notify]);

  useEffect(() => {
    if (started.current) return;
    started.current = true;
    if (!api) { setBootError("无法连接桌面服务。请通过 omniTerm 桌面应用打开此窗口。"); return; }
    function readWorkspace() {
      try { return parseWorkspace(localStorage.getItem(WORKSPACE_KEY)); }
      catch { return null; }
    }
    // Opens a fresh shell for every saved pane. A directory that no longer
    // exists falls back to the default one instead of losing the pane.
    async function restoreWorkspace(snapshot, fallbackCwd) {
      const opened = [];
      let missing = 0;
      const openPane = async (directory) => {
        for (const candidate of directory && directory !== fallbackCwd ? [directory, fallbackCwd] : [fallbackCwd]) {
          try {
            const session = await api.createSession({ kind: "shell", cwd: candidate });
            if (candidate !== directory && directory) missing++;
            opened.push({ ...session, title: `本地终端 ${nextTerminal.current++}` });
            return session.id;
          } catch { /* Try the default directory, then give up on this pane. */ }
        }
        return null;
      };
      const restored = [];
      for (const saved of snapshot.tabs) {
        const root = await restoreTree(saved.root, openPane);
        if (root) restored.push({ id: newTabId(), root, focus: paneIds(root)[0], title: saved.title, broadcast: false, zoomed: null });
      }
      if (!restored.length) return false;
      const active = restored[Math.min(snapshot.active, restored.length - 1)];
      setSessions(opened); setTabs(restored); setActiveTabId(active.id); setFocusedId(active.focus);
      if (missing) notify(`${missing} 个窗格的上次目录已不存在，已在默认目录打开。`);
      return true;
    }
    async function initialize() {
      try {
        const details = await api.getAppInfo();
        setInfo(details); setCwd(details.cwd || details.home); setProfiles(details.profiles || []);
        const fallbackCwd = details.cwd || details.home;
        const snapshot = readSettings().restoreWorkspace ? readWorkspace() : null;
        if (!snapshot || !await restoreWorkspace(snapshot, fallbackCwd)) {
          const session = await api.createSession({ kind: "shell", cwd: fallbackCwd });
          const tab = { id: newTabId(), root: pane(session.id), focus: session.id, title: "", broadcast: false, zoomed: null };
          setSessions([{ ...session, title: "本地终端" }]); setTabs([tab]); setActiveTabId(tab.id); setFocusedId(session.id);
        }
        // A failed start must not overwrite the saved layout with an empty one.
        ready.current = true;
      } catch (error) { setBootError(error.message || String(error)); }
    }
    initialize();
    return () => clearTimeout(toastTimer.current);
  }, []);

  // Saved continuously so a crash or force-quit still restores the last layout.
  useEffect(() => {
    if (!ready.current) return;
    try {
      if (settings.restoreWorkspace) localStorage.setItem(WORKSPACE_KEY, JSON.stringify(serializeWorkspace(tabs, sessions, activeTabId)));
      else localStorage.removeItem(WORKSPACE_KEY);
    } catch { /* Restoring is best effort; the current session is unaffected. */ }
  }, [tabs, sessions, activeTabId, settings.restoreWorkspace]);

  useEffect(() => {
    try { localStorage.setItem("omniterm-terminal-settings", JSON.stringify(settings)); }
    catch { notify("无法保存终端外观设置，本次会话中仍然生效。"); }
  }, [settings, notify]);

  const activeTab = tabs.find((tab) => tab.id === activeTabId);
  const activeSession = sessions.find((session) => session.id === activeTab?.focus);
  const agentFocused = Boolean(agentSession && agentVisible && focusedId === agentSession.id);
  const selectedSession = agentFocused ? agentSession : activeSession;
  const searchId = selectedSession?.id;
  const theme = themeFor(settings.theme);
  const modifier = info?.platform === "darwin" ? "⌘" : "Ctrl";
  const key = (name) => shortcutLabel(name, info?.platform);

  const runControl = async (action, id = searchId) => {
    try { await controls.current.get(id)?.[action](); }
    catch (error) { notify(error.message); }
  };
  const showSearch = useCallback(() => { if (!selectedSession) return; setSearchOpen(true); requestAnimationFrame(() => { searchInput.current?.focus(); searchInput.current?.select(); }); }, [selectedSession]);
  const hideSearch = () => { setSearchOpen(false); controls.current.get(searchId)?.clearSearch(); controls.current.get(searchId)?.focus(); };
  const find = (value = searchValue, previous = false, options = searchOptions) => {
    try { setSearchFound(controls.current.get(searchId)?.find(value, previous, options) ?? true); }
    catch { setSearchFound(false); }
  };

  async function chooseDirectory() {
    try {
      const directory = await api.chooseDirectory();
      if (directory) { setCwd(directory); await addTab("shell", undefined, directory); }
    } catch (error) { notify(error.message); }
  }
  async function startAgent() {
    if (agentStarting) return;
    setAgentStarting(true);
    try {
      if (agentSession) { await api.closeSession(agentSession.id); setAgentSession(null); }
      // Start where the user is working: the focused shell's live directory.
      const directory = activeSession?.kind === "shell" ? activeSession.liveCwd || activeSession.cwd : cwd;
      const session = await api.createSession({ kind: "agent", cwd: directory });
      setAgentSession({ ...session, title: "Codex Agent" }); setAgentVisible(true); setFocusedId(session.id);
    } catch (error) { notify(error.message); }
    finally { setAgentStarting(false); }
  }
  const sendSelectionToAgent = useCallback(() => {
    const sourceId = activeTab?.focus;
    const selection = controls.current.get(sourceId)?.getSelection()?.trim();
    if (!selection) { notify("请先在终端中选择要发送给 Agent 的内容。"); return; }
    if (!agentSession || finished(agentSession.status)) { notify("请先启动 Agent，再发送选中内容。"); setAgentVisible(true); return; }
    const source = sessionsRef.current.find((session) => session.id === sourceId);
    try {
      // Only the explicitly selected text is shared, never the whole buffer.
      controls.current.get(agentSession.id)?.pasteText(`以下内容来自终端「${source?.termTitle || source?.title || "终端"}」：\n\`\`\`\n${selection}\n\`\`\`\n`);
      setAgentVisible(true); setFocusedId(agentSession.id); focusLater(agentSession.id);
      notify("已将选中内容发送到 Agent 输入框，补充问题后按回车提交。", "success");
    } catch (error) { notify(error.message); }
  }, [activeTab, agentSession, notify, focusLater]);
  const toggleBroadcast = useCallback(() => {
    if (!activeTab) return;
    const enabled = !activeTab.broadcast;
    setTabs((previous) => previous.map((tab) => tab.id === activeTab.id ? { ...tab, broadcast: enabled } : tab));
    notify(enabled ? "广播输入已开启：键盘输入会发送到此标签内的所有窗格。" : "广播输入已关闭。", "success");
  }, [activeTab, notify]);
  const toggleZoom = useCallback(() => {
    if (!activeTab || paneIds(activeTab.root).length < 2) return;
    setTabs((previous) => previous.map((tab) => tab.id === activeTab.id ? { ...tab, zoomed: tab.zoomed ? null : tab.focus } : tab));
    focusLater(activeTab.focus);
  }, [activeTab, focusLater]);
  const cyclePane = useCallback((step) => {
    if (!activeTab) return;
    const ids = paneIds(activeTab.root);
    const next = ids[(ids.indexOf(activeTab.focus) + step + ids.length) % ids.length];
    if (activeTab.zoomed) setTabs((previous) => previous.map((tab) => tab.id === activeTab.id ? { ...tab, zoomed: next } : tab));
    activatePane(next);
  }, [activeTab, activatePane]);
  const movePane = useCallback((direction) => {
    if (!activeTab) return;
    if (agentFocused) { if (direction === "left") activatePane(activeTab.focus); return; }
    const next = neighborPane(activeTab.root, activeTab.focus, direction);
    if (next) activatePane(next);
    else if (direction === "right" && agentSession && agentVisible) { setFocusedId(agentSession.id); focusLater(agentSession.id); }
  }, [activeTab, agentFocused, agentSession, agentVisible, activatePane, focusLater]);
  const selectTab = useCallback((tabId) => {
    const tab = tabsRef.current.find((item) => item.id === tabId);
    if (!tab) return;
    setActiveTabId(tab.id); setFocusedId(tab.focus); focusLater(tab.focus);
  }, [focusLater]);
  const renameTab = (tabId, title) => setTabs((previous) => previous.map((tab) => tab.id === tabId ? { ...tab, title } : tab));
  const reorderTab = (fromId, toId) => setTabs((previous) => {
    const from = previous.findIndex((tab) => tab.id === fromId);
    const to = previous.findIndex((tab) => tab.id === toId);
    if (from < 0 || to < 0 || from === to) return previous;
    const next = [...previous];
    next.splice(to, 0, next.splice(from, 1)[0]);
    return next;
  });
  const fontSize = (delta) => setSettings((previous) => ({ ...previous, fontSize: delta ? Math.min(28, Math.max(9, previous.fontSize + delta)) : defaults.fontSize }));
  const safely = (task) => Promise.resolve().then(task).catch((error) => notify(error.message || String(error)));

  const actions = {
    newTab: () => safely(() => addTab()),
    closePane: () => selectedSession && closePane(selectedSession.id),
    splitRight: () => safely(() => splitFocused("row")),
    splitDown: () => safely(() => splitFocused("column")),
    nextPane: () => cyclePane(1),
    previousPane: () => cyclePane(-1),
    paneLeft: () => movePane("left"),
    paneRight: () => movePane("right"),
    paneUp: () => movePane("up"),
    paneDown: () => movePane("down"),
    maximizePane: toggleZoom,
    find: showSearch,
    clear: () => runControl("clear"),
    palette: () => setPaletteOpen(true),
    broadcast: toggleBroadcast,
    sendToAgent: sendSelectionToAgent,
    toggleAgent: () => setAgentVisible((visible) => !visible),
    fontBigger: () => fontSize(1),
    fontSmaller: () => fontSize(-1),
    fontReset: () => fontSize(0),
    settings: () => setSettingsOpen(true),
  };
  const actionsRef = useRef(actions);
  actionsRef.current = actions;

  useEffect(() => {
    const handler = (event) => {
      if (dialog || settingsOpen || deleting || paletteOpen || !info) return;
      const cycle = tabCycle(event);
      const number = tabNumber(event, info.platform);
      const name = matchShortcut(event, info.platform);
      if (!cycle && !number && !name) return;
      event.preventDefault();
      if (name) { actionsRef.current[name](); return; }
      const list = tabsRef.current;
      if (!list.length) return;
      if (number) { const tab = number === 9 ? list.at(-1) : list[number - 1]; if (tab) selectTab(tab.id); return; }
      const index = list.findIndex((tab) => tab.id === activeTabId);
      selectTab(list[(index + cycle + list.length) % list.length].id);
    };
    window.addEventListener("keydown", handler);
    return () => window.removeEventListener("keydown", handler);
  }, [dialog, settingsOpen, deleting, paletteOpen, info, activeTabId, selectTab]);

  // Built only while the palette is open so every command sees current state.
  const commands = !info || !paletteOpen ? [] : [
      { id: "new-tab", group: "终端", label: "新建本地终端标签", icon: Plus, shortcut: key("newTab"), run: actions.newTab },
      { id: "split-right", group: "窗格", label: "向右分屏", icon: SquareSplitHorizontal, shortcut: key("splitRight"), run: actions.splitRight },
      { id: "split-down", group: "窗格", label: "向下分屏", icon: SquareSplitVertical, shortcut: key("splitDown"), run: actions.splitDown },
      { id: "zoom", group: "窗格", label: activeTab?.zoomed ? "还原窗格" : "最大化当前窗格", icon: activeTab?.zoomed ? ArrowsIn : ArrowsOut, shortcut: key("maximizePane"), run: actions.maximizePane, disabled: !activeTab || paneIds(activeTab.root).length < 2 },
      { id: "equalize", group: "窗格", label: "平均分配窗格大小", icon: SquareSplitHorizontal, run: () => activeTab && setTabs((previous) => previous.map((tab) => tab.id === activeTab.id ? { ...tab, root: equalize(tab.root) } : tab)), disabled: !activeTab },
      { id: "broadcast", group: "窗格", label: activeTab?.broadcast ? "关闭广播输入" : "开启广播输入（同时输入到所有窗格）", icon: Broadcast, shortcut: key("broadcast"), run: actions.broadcast, disabled: !activeTab },
      { id: "close-pane", group: "终端", label: "关闭当前窗格", icon: X, shortcut: key("closePane"), run: actions.closePane, disabled: !selectedSession },
      { id: "find", group: "终端", label: "搜索输出", icon: MagnifyingGlass, shortcut: key("find"), run: actions.find, disabled: !selectedSession },
      { id: "clear", group: "终端", label: "清空屏幕", icon: Trash, shortcut: key("clear"), run: actions.clear, disabled: !selectedSession },
      { id: "select-all", group: "终端", label: "全选输出", icon: Copy, run: () => runControl("selectAll"), disabled: !selectedSession },
      { id: "open-folder", group: "工作区", label: "在文件夹中打开终端…", icon: FolderOpen, run: chooseDirectory },
      { id: "agent-start", group: "Agent", label: agentSession ? "重新启动 Agent" : "启动 Agent", icon: Robot, run: startAgent, disabled: !info.agent?.available },
      { id: "agent-send", group: "Agent", label: "发送选中内容到 Agent", icon: PaperPlaneTilt, shortcut: key("sendToAgent"), run: actions.sendToAgent },
      { id: "agent-toggle", group: "Agent", label: agentVisible ? "隐藏 Agent 面板" : "显示 Agent 面板", icon: SidebarSimple, shortcut: key("toggleAgent"), run: actions.toggleAgent },
      { id: "font-bigger", group: "外观", label: "放大字体", icon: TextAa, shortcut: key("fontBigger"), run: actions.fontBigger },
      { id: "font-smaller", group: "外观", label: "缩小字体", icon: TextAa, shortcut: key("fontSmaller"), run: actions.fontSmaller },
      { id: "font-reset", group: "外观", label: "重置字体大小", icon: TextAa, shortcut: key("fontReset"), run: actions.fontReset },
      ...Object.entries(THEMES).map(([id, item]) => ({ id: `theme-${id}`, group: "主题", label: item.name, keywords: "theme color 配色", icon: Palette, run: () => setSettings((previous) => ({ ...previous, theme: id })) })),
      { id: "settings", group: "设置", label: "打开终端设置", icon: GearSix, shortcut: key("settings"), run: actions.settings },
      ...Object.entries(connectionLabels).map(([kind, label]) => ({ id: `new-${kind}`, group: "新建连接", label, keywords: kind, icon: kindIcons[kind], run: () => setDialog({ kind }) })),
      ...profiles.map((profile) => ({ id: `profile-${profile.id}`, group: "连接", label: profile.name, keywords: `${profile.kind} ${profile.host || ""} ${profile.device || ""}`, icon: kindIcons[profile.kind] || HardDrives, run: () => safely(() => addTab(profile.kind, profile)) })),
      ...tabs.map((tab, index) => {
        const focused = sessions.find((session) => session.id === tab.focus);
        return { id: `tab-${tab.id}`, group: "标签", label: `${index + 1}. ${tab.title || focused?.termTitle || focused?.title || "终端"}`, icon: kindIcons[focused?.kind] || TerminalWindow, run: () => selectTab(tab.id) };
      }),
  ];

  const runningCount = sessions.filter((session) => !finished(session.status)).length + (agentSession && !finished(agentSession.status) ? 1 : 0);
  const paneCount = activeTab ? paneIds(activeTab.root).length : 0;

  const layouts = useMemo(() => new Map(tabs.map((tab) => [tab.id, computeLayout(tab.root)])), [tabs]);
  const placement = useMemo(() => {
    const result = new Map();
    for (const tab of tabs) {
      for (const item of layouts.get(tab.id).panes) {
        const zoomedOut = tab.zoomed && tab.zoomed !== item.sessionId;
        result.set(item.sessionId, { tab, style: tab.zoomed && !zoomedOut ? paneStyle({ x: 0, y: 0, w: 1, h: 1 }) : paneStyle(item.rect), hidden: Boolean(zoomedOut) });
      }
    }
    return result;
  }, [tabs, layouts]);

  const startDividerDrag = (event, tabId, divider) => {
    event.preventDefault();
    const stack = stackRef.current?.getBoundingClientRect();
    if (!stack) return;
    const target = event.currentTarget;
    target.setPointerCapture(event.pointerId);
    const move = (moveEvent) => {
      const row = divider.direction === "row";
      const fraction = row ? (moveEvent.clientX - stack.left) / stack.width : (moveEvent.clientY - stack.top) / stack.height;
      const ratio = row ? (fraction - divider.parent.x) / divider.parent.w : (fraction - divider.parent.y) / divider.parent.h;
      setTabs((previous) => previous.map((tab) => tab.id === tabId ? { ...tab, root: setRatio(tab.root, divider.path, ratio) } : tab));
    };
    const stop = () => { target.removeEventListener("pointermove", move); target.removeEventListener("pointerup", stop); target.removeEventListener("pointercancel", stop); };
    target.addEventListener("pointermove", move);
    target.addEventListener("pointerup", stop);
    target.addEventListener("pointercancel", stop);
  };

  if (!info) return <div className="app-boot"><div className="boot-logo"><TerminalWindow size={32} /></div><h1>omniTerm</h1>{bootError ? <><p className="boot-error">{bootError}</p><button className="button secondary" onClick={() => window.location.reload()}><ArrowClockwise size={16} />重新连接</button></> : <p><CircleNotch size={16} className="spin" />正在打开工作区…</p>}</div>;

  const tabLabel = (tab) => {
    const focused = sessions.find((session) => session.id === tab.focus);
    return tab.title || focused?.termTitle || focused?.title || "终端";
  };
  const viewProps = { settings, platform: info.platform, api, register, onState, onError: notify, onInput, onCwd, onTitle };
  const shownCwd = activeSession?.liveCwd || activeSession?.cwd || cwd;

  return <div className={`app-shell platform-${info.platform} ${theme.light ? "terminal-light" : ""}`} style={{ "--term-bg": theme.chrome }}>
    <header className="titlebar">
      <div className="window-brand"><TerminalWindow size={19} weight="duotone" /><strong>omniTerm</strong></div>
      <div className="window-title"><FolderSimple size={13} /><span>{basename(shownCwd)}</span><span className="title-divider">/</span><span>{activeTab ? tabLabel(activeTab) : "工作区"}</span></div>
      <div className="titlebar-actions">
        <button className="titlebar-button" title={`命令面板（${key("palette")}）`} onClick={() => setPaletteOpen(true)}><Command size={15} /><span>命令</span><kbd>{key("palette")}</kbd></button>
        <button className={`titlebar-button ${agentVisible ? "active" : ""}`} title={`${agentVisible ? "隐藏 Agent 面板（会话继续运行）" : "显示 Agent 面板"}（${key("toggleAgent")}）`} onClick={() => setAgentVisible((visible) => !visible)}><Robot size={16} /><span>Agent</span><SidebarSimple size={15} /></button>
      </div>
    </header>

    <div className="workspace-layout">
      <aside className="sidebar">
        <div className="sidebar-section-heading"><span>工作区</span><IconButton label="打开工作文件夹" onClick={chooseDirectory}><FolderOpen size={16} /></IconButton></div>
        <button className="workspace-switcher" onClick={chooseDirectory} title={cwd}><span className="workspace-avatar"><FolderSimple size={20} /></span><span><strong>{basename(cwd)}</strong><small>{pathLabel(cwd, info.home)}</small></span><CaretRight size={13} /></button>
        <nav className="workspace-nav" aria-label="工作区导航">
          <button className={`nav-item ${!agentFocused && selectedSession?.kind === "shell" ? "active" : ""}`} onClick={() => { const tab = tabs.find((item) => paneIds(item.root).some((id) => sessions.find((session) => session.id === id)?.kind === "shell")); if (tab) selectTab(tab.id); else safely(() => addTab()); }}><TerminalWindow size={18} /><span>本地终端</span><span className="nav-count">{sessions.filter((session) => session.kind === "shell").length}</span></button>
          <button className={`nav-item ${agentFocused ? "active" : ""}`} onClick={() => { setAgentVisible(true); if (agentSession) { setFocusedId(agentSession.id); focusLater(agentSession.id); } }}><Robot size={18} /><span>Agent</span>{agentSession?.status === "running" ? <span className="status-dot" /> : <span className="nav-meta">Codex</span>}</button>
        </nav>
        <div className="sidebar-section-heading saved-heading"><span>已保存的连接</span><IconButton label="添加连接" onClick={() => { setProfileMenu(null); setDialog({ kind: "ssh" }); }}><Plus size={15} /></IconButton></div>
        <div className="saved-connections">
          {profiles.length ? profiles.map((profile) => {
            const Icon = kindIcons[profile.kind] || HardDrives;
            return <div className="profile-wrap" key={profile.id}><div className="profile-row"><button className="profile-connect" onClick={() => safely(() => addTab(profile.kind, profile))} title={`连接 ${profile.name}`}><Icon size={17} /><span><strong>{profile.name}</strong><small>{profile.kind === "serial" ? profile.device : `${profile.username ? `${profile.username}@` : ""}${profile.host}`}</small></span></button><IconButton label={`${profile.name}的选项`} onClick={() => setProfileMenu(profileMenu === profile.id ? null : profile.id)}><DotsThree size={19} /></IconButton></div>{profileMenu === profile.id && <div className="profile-menu"><button onClick={() => { setProfileMenu(null); setDialog({ kind: profile.kind, profile }); }}><PencilSimple size={14} />编辑连接</button><button onClick={() => { setProfileMenu(null); setDeleting(profile); }}><Trash size={14} />删除连接</button></div>}</div>;
          }) : <div className="connections-empty"><HardDrives size={23} /><p>你的连接，会在这里</p><span>保存常用主机，一键连接。</span><button className="text-button" onClick={() => setDialog({ kind: "ssh" })}><Plus size={13} />添加第一个连接</button></div>}
        </div>
        <div className="sidebar-bottom"><div className="sidebar-section-heading"><span>连接工具</span></div><div className="connection-tools">{Object.entries(connectionLabels).map(([kind, label]) => { const Icon = kindIcons[kind]; return <button key={kind} onClick={() => { setProfileMenu(null); setDialog({ kind }); }}><Icon size={16} /><span>{label}</span></button>; })}</div><button className="settings-button" onClick={() => setSettingsOpen(true)}><GearSix size={18} /><span>设置</span><span className="version-label">v{info.version}</span></button></div>
      </aside>

      <main className="main-workspace">
        <div className="workspace-toolbar"><div className="breadcrumb" title={shownCwd}><FolderSimple size={16} /><span>工作区</span><CaretRight size={12} /><strong>{pathLabel(shownCwd, info.home)}</strong></div><button className="button toolbar-button" onClick={chooseDirectory}><FolderOpen size={15} />打开文件夹</button></div>
        <div className={`panes ${agentVisible ? "with-agent" : ""}`}>
          <section className="terminal-pane" aria-label="终端工作区">
            <div className="tabstrip"><div className="tabs" role="tablist" aria-label="终端会话">{tabs.map((tab, index) => {
              const focused = sessions.find((session) => session.id === tab.focus);
              const Icon = kindIcons[focused?.kind] || TerminalWindow;
              const ids = paneIds(tab.root);
              const allDone = ids.every((id) => finished(sessions.find((session) => session.id === id)?.status));
              return <div key={tab.id} className={`tab ${activeTabId === tab.id ? "active" : ""} ${draggedTab === tab.id ? "dragging" : ""}`} draggable onDragStart={(event) => { setDraggedTab(tab.id); event.dataTransfer.effectAllowed = "move"; event.dataTransfer.setData("text/omniterm-tab", tab.id); }} onDragEnd={() => setDraggedTab(null)} onDragOver={(event) => { if (draggedTab) event.preventDefault(); }} onDrop={(event) => { event.preventDefault(); if (draggedTab) reorderTab(draggedTab, tab.id); setDraggedTab(null); }} onAuxClick={(event) => { if (event.button === 1) closeTab(tab.id); }}>
                <button role="tab" aria-selected={activeTabId === tab.id} onClick={() => selectTab(tab.id)} title={`${tabLabel(tab)}${index < 9 ? `（${info.platform === "darwin" ? "⌘" : "Alt+"}${index + 1}）` : ""}`}>
                  <Icon size={15} />
                  <TabTitle tab={tab} title={tabLabel(tab)} onRename={renameTab} />
                  {ids.length > 1 && <span className="tab-badge" title={`${ids.length} 个窗格`}>{ids.length}</span>}
                  {tab.broadcast && <Broadcast size={12} className="broadcast-indicator" />}
                  {allDone && <Circle size={6} weight="fill" className="inactive-indicator" />}
                </button>
                <IconButton label={`关闭 ${tabLabel(tab)}`} onClick={() => closeTab(tab.id)}><X size={12} /></IconButton>
              </div>;
            })}</div><IconButton className="new-tab" label={`新建终端（${key("newTab")}）`} onClick={() => safely(() => addTab())}><Plus size={17} /></IconButton></div>
            <div className="terminal-toolbar">
              <div className="session-detail">{activeSession ? <><span className={`status-dot ${activeSession.status === "running" ? "" : "muted"}`} /><span>{activeSession.kind === "shell" ? "本地 Shell" : connectionLabels[activeSession.kind]}</span><span className="toolbar-dot">·</span><span>{activeSession.status === "exited" ? "会话已结束" : activeSession.status === "error" ? "启动失败" : activeSession.status === "running" ? "已连接" : "正在连接"}</span>{paneCount > 1 && <><span className="toolbar-dot">·</span><span>{paneCount} 个窗格{activeTab.zoomed ? "（已最大化）" : ""}</span></>}{activeTab?.broadcast && <span className="broadcast-pill"><Broadcast size={12} />广播输入</span>}</> : <span>终端</span>}</div>
              <div className="terminal-actions">
                <IconButton label={`向右分屏（${key("splitRight")}）`} onClick={actions.splitRight} disabled={!activeTab}><SquareSplitHorizontal size={15} /></IconButton>
                <IconButton label={`向下分屏（${key("splitDown")}）`} onClick={actions.splitDown} disabled={!activeTab}><SquareSplitVertical size={15} /></IconButton>
                <IconButton label={`${activeTab?.zoomed ? "还原" : "最大化"}窗格（${key("maximizePane")}）`} onClick={toggleZoom} disabled={paneCount < 2}>{activeTab?.zoomed ? <ArrowsIn size={15} /> : <ArrowsOut size={15} />}</IconButton>
                <IconButton className={activeTab?.broadcast ? "is-active" : ""} label={`广播输入到所有窗格（${key("broadcast")}）`} onClick={toggleBroadcast} disabled={!activeTab}><Broadcast size={15} /></IconButton>
                <span className="toolbar-separator" />
                <IconButton label={`搜索终端（${key("find")}）`} onClick={showSearch} disabled={!selectedSession}><MagnifyingGlass size={15} /></IconButton>
                <IconButton label="复制选中文本" onClick={() => runControl("copy")} disabled={!selectedSession}><Copy size={15} /></IconButton>
                <IconButton label="粘贴到终端" onClick={() => runControl("paste")} disabled={!selectedSession}><ClipboardText size={15} /></IconButton>
                <IconButton label={`发送选中内容到 Agent（${key("sendToAgent")}）`} onClick={sendSelectionToAgent} disabled={!activeSession}><PaperPlaneTilt size={15} /></IconButton>
                <IconButton label={`清空终端屏幕（${key("clear")}）`} onClick={() => runControl("clear")} disabled={!selectedSession}><Trash size={15} /></IconButton>
              </div>
            </div>
            {searchOpen && <div className="search-bar"><MagnifyingGlass size={15} /><input ref={searchInput} value={searchValue} placeholder={`搜索${selectedSession?.kind === "agent" ? " Agent" : "终端"}输出`} onChange={(event) => { setSearchValue(event.target.value); find(event.target.value); }} onKeyDown={(event) => { if (event.key === "Enter") { event.preventDefault(); find(searchValue, event.shiftKey); } if (event.key === "Escape") hideSearch(); }} /><span className="search-feedback">{searchValue && !searchFound ? "未找到" : ""}</span>
              <button type="button" className={`search-option ${searchOptions.caseSensitive ? "on" : ""}`} title="区分大小写" aria-pressed={searchOptions.caseSensitive} onClick={() => { const next = { ...searchOptions, caseSensitive: !searchOptions.caseSensitive }; setSearchOptions(next); find(searchValue, false, next); }}>Aa</button>
              <button type="button" className={`search-option ${searchOptions.regex ? "on" : ""}`} title="正则表达式" aria-pressed={searchOptions.regex} onClick={() => { const next = { ...searchOptions, regex: !searchOptions.regex }; setSearchOptions(next); find(searchValue, false, next); }}>.*</button>
              <IconButton label="上一个结果" onClick={() => find(searchValue, true)}><ArrowUp size={14} /></IconButton><IconButton label="下一个结果" onClick={() => find()}><ArrowDown size={14} /></IconButton><IconButton label="关闭搜索" onClick={hideSearch}><X size={14} /></IconButton></div>}
            <div className="terminal-stack" ref={stackRef}>
              {sessions.map((session) => {
                const place = placement.get(session.id);
                if (!place) return null;
                const visible = place.tab.id === activeTabId && !place.hidden;
                const multiple = paneIds(place.tab.root).length > 1 && !place.tab.zoomed;
                const isFocused = multiple && place.tab.focus === session.id && !agentFocused;
                return <TerminalView key={session.id} session={session} visible={visible} focused={isFocused} dimmed={multiple && settings.dimInactive && place.tab.focus !== session.id} style={place.style} onFocus={focusPane} {...viewProps} />;
              })}
              {activeTab && !activeTab.zoomed && layouts.get(activeTab.id)?.dividers.map((divider) => {
                const row = divider.direction === "row";
                const style = row
                  ? { left: `calc(${divider.position * 100}% - ${DIVIDER / 2}px)`, top: `${divider.parent.y * 100}%`, width: DIVIDER, height: `${divider.parent.h * 100}%` }
                  : { top: `calc(${divider.position * 100}% - ${DIVIDER / 2}px)`, left: `${divider.parent.x * 100}%`, height: DIVIDER, width: `${divider.parent.w * 100}%` };
                return <div key={divider.path.join("-") || "root"} className={`pane-divider ${row ? "row" : "column"}`} style={style} role="separator" aria-orientation={row ? "vertical" : "horizontal"} title="拖动调整大小，双击平均分配" onPointerDown={(event) => startDividerDrag(event, activeTab.id, divider)} onDoubleClick={() => setTabs((previous) => previous.map((tab) => tab.id === activeTab.id ? { ...tab, root: setRatio(tab.root, divider.path, 0.5) } : tab))} />;
              })}
              {!tabs.length && <div className="terminal-empty"><div className="empty-terminal-icon"><TerminalWindow size={34} weight="light" /></div><h2>开始一个新的会话</h2><p>打开本地终端，或连接到你的远程主机。</p><button className="button primary" onClick={() => safely(() => addTab())}><Plus size={16} />新建终端<kbd>{key("newTab")}</kbd></button><p className="empty-hint">按 <kbd>{key("palette")}</kbd> 打开命令面板</p></div>}
            </div>
            <div className="terminal-footnote"><span>{pathLabel(shownCwd, info.home)}</span><span>{theme.name} · {settings.gpu ? "GPU" : "DOM"} · UTF-8</span></div>
          </section>

          <aside className={`agent-pane ${agentVisible ? "is-visible" : ""} ${agentFocused ? "is-focused" : ""}`} aria-label="Agent 面板">
            <div className="agent-heading"><Robot size={18} /><strong>Agent</strong><span className="agent-model-label">Codex</span><div className="agent-heading-actions">{agentSession && <IconButton label={`发送终端选中内容（${key("sendToAgent")}）`} onClick={sendSelectionToAgent}><PaperPlaneTilt size={15} /></IconButton>}{agentSession && <IconButton label="结束 Agent 会话" onClick={() => closePane(agentSession.id)}><Trash size={15} /></IconButton>}<IconButton label="隐藏 Agent 面板" onClick={() => setAgentVisible(false)}><SidebarSimple size={17} /></IconButton></div></div>
            <div className="agent-context"><FolderSimple size={14} /><span title={agentSession?.cwd || cwd}>{basename(agentSession?.cwd || cwd)}</span><span className="context-local">本地工作区</span></div>
            {agentSession ? <div className="agent-terminal-stack"><TerminalView session={agentSession} visible={agentVisible} onFocus={setFocusedId} {...viewProps} />{finished(agentSession.status) && <div className="agent-restart"><button className="button secondary" onClick={startAgent} disabled={agentStarting}><ArrowClockwise size={15} />重新启动 Agent</button></div>}</div> : <div className="agent-invitation"><div className="agent-symbol"><Robot size={32} weight="light" /><span className={`agent-availability ${info.agent?.available ? "available" : ""}`} /></div><span className="agent-eyebrow">你的终端搭档</span><h2>从一个想法开始。</h2><p>让 Agent 理解代码、处理任务。<br />你掌握每一步的决定权。</p><button className="button primary launch-agent" onClick={startAgent} disabled={!info.agent?.available || agentStarting}>{agentStarting ? <CircleNotch className="spin" size={17} /> : <Robot size={17} />}启动 Agent<ArrowRight size={15} /></button><div className="agent-local-note"><Cpu size={13} /><span>{info.agent?.available ? "Codex 运行于本机，在当前终端目录启动" : "本地 Agent 暂不可用"}</span></div>{!info.agent?.available && <div className="agent-unavailable"><Info size={15} /><span>{info.agent?.error || "未找到 Agent 程序，请完成本地 Agent 安装。"}</span></div>}<div className="agent-explanation"><div><Code size={17} /><span>理解当前项目与终端环境</span></div><div><PaperPlaneTilt size={17} /><span>选中终端输出，按 {key("sendToAgent")} 发给 Agent</span></div><div><TerminalWindow size={17} /><span>编写代码，执行经你批准的操作</span></div></div></div>}
            <div className="agent-footer"><span className={`status-dot ${info.agent?.available ? "" : "muted"}`} /><span>{agentSession?.status === "running" ? "Agent 会话运行中" : "由本地 Codex 提供支持"}</span><span className="agent-footer-label">Agent 模式</span></div>
          </aside>
        </div>
      </main>
    </div>
    <footer className="statusbar"><div><Monitor size={13} /><span>{platformNames[info.platform] || info.platform}</span><span className="statusbar-separator" /><span className="status-dot" /><span>{runningCount} 个活动会话</span>{activeTab?.broadcast && <><span className="statusbar-separator" /><Broadcast size={12} /><span>广播输入中</span></>}</div><div><span className={`status-dot ${info.agent?.available ? "" : "muted"}`} /><span>本地 Agent {info.agent?.available ? "已就绪" : "未就绪"}</span><span className="statusbar-separator" /><button onClick={() => setPaletteOpen(true)} title="命令面板"><Command size={12} /><span>{key("palette")}</span></button></div></footer>
    {dialog && <ConnectionDialog initial={dialog} onClose={() => setDialog(null)} onSave={async (profile) => setProfiles(await api.saveProfile(profile))} onConnect={addTab} />}
    {settingsOpen && <SettingsDialog settings={settings} setSettings={setSettings} info={info} onClose={() => setSettingsOpen(false)} />}
    {paletteOpen && <CommandPalette commands={commands} onClose={() => { setPaletteOpen(false); if (selectedSession) focusLater(selectedSession.id); }} />}
    {deleting && <DeleteDialog profile={deleting} onClose={() => setDeleting(null)} onConfirm={async (id) => setProfiles(await api.deleteProfile(id))} />}
    {toast && <div className={`toast ${toast.type}`} role="alert">{toast.type === "error" ? <WarningCircle size={19} /> : <Check size={19} />}<span>{toast.message}</span><IconButton label="关闭提示" onClick={() => setToast(null)}><X size={15} /></IconButton></div>}
    {bootError && info && <div className="toast error" role="alert"><WarningCircle size={19} /><span>{bootError}</span><IconButton label="关闭启动提示" onClick={() => setBootError("")}><X size={15} /></IconButton></div>}
  </div>;
}
