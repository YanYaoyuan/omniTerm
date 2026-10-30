import { useCallback, useEffect, useRef, useState } from "react";
import {
  ArrowDown, ArrowRight, ArrowUp, ArrowsLeftRight, CaretRight, Check, Circle,
  CircleNotch, Code, Command, Copy, Cpu, DotsThree, FolderOpen, FolderSimple,
  GearSix, HardDrives, Info, MagnifyingGlass, Monitor, PencilSimple,
  PlugsConnected, Plus, Robot, SidebarSimple, TerminalWindow, Trash, X,
  ClipboardText, WarningCircle, SlidersHorizontal, ArrowClockwise,
} from "@phosphor-icons/react";
import { TerminalView } from "./TerminalView.jsx";
import { ConnectionDialog, connectionLabels } from "./ConnectionDialog.jsx";

const api = window.omniterm;
const defaults = { fontSize: 13, scrollback: 10000, cursorBlink: true };
const kindIcons = { shell: TerminalWindow, agent: Robot, ssh: HardDrives, sftp: FolderOpen, serial: PlugsConnected, tunnel: ArrowsLeftRight };
const platformNames = { darwin: "macOS", win32: "Windows", linux: "Linux" };
const basename = (value = "") => value.split(/[/\\]/).filter(Boolean).at(-1) || value;
const pathLabel = (value, home) => value && home && value.startsWith(home) ? `~${value.slice(home.length)}` : value;

function readSettings() {
  try { return { ...defaults, ...JSON.parse(localStorage.getItem("omniterm-terminal-settings") || "{}") }; }
  catch { return defaults; }
}

function IconButton({ label, children, className = "", ...props }) {
  return <button type="button" className={`icon-button ${className}`} title={label} aria-label={label} {...props}>{children}</button>;
}

function SettingsDialog({ settings, setSettings, info, onClose }) {
  const dialog = useRef(null);
  useEffect(() => { dialog.current.showModal(); }, []);
  return <dialog className="modal settings-modal" ref={dialog} onCancel={(event) => { event.preventDefault(); onClose(); }} onClick={(event) => { if (event.target === dialog.current) onClose(); }}>
    <div className="modal-heading"><div className="modal-heading-icon"><SlidersHorizontal size={22} /></div><div><h2>终端设置</h2><p>让工作区更适合你的习惯</p></div><IconButton className="modal-close" label="关闭设置" onClick={onClose}><X size={18} /></IconButton></div>
    <div className="settings-content">
      <div className="setting-row"><div><strong>字体大小</strong><span>应用到所有终端和 Agent 会话</span></div><div className="number-stepper"><button onClick={() => setSettings((s) => ({ ...s, fontSize: Math.max(10, s.fontSize - 1) }))} disabled={settings.fontSize <= 10} aria-label="缩小字体">−</button><span>{settings.fontSize} px</span><button onClick={() => setSettings((s) => ({ ...s, fontSize: Math.min(22, s.fontSize + 1) }))} disabled={settings.fontSize >= 22} aria-label="放大字体">+</button></div></div>
      <div className="font-preview" style={{ fontSize: settings.fontSize }}>omniTerm ~/workspace $<span className="sample-cursor" /></div>
      <div className="setting-row"><div><strong>回滚缓冲区</strong><span>保留每个终端的历史输出行数</span></div><select value={settings.scrollback} onChange={(event) => setSettings((s) => ({ ...s, scrollback: Number(event.target.value) }))}><option value="1000">1,000 行</option><option value="5000">5,000 行</option><option value="10000">10,000 行</option><option value="50000">50,000 行</option></select></div>
      <div className="setting-row"><div><strong>光标闪烁</strong><span>在输入位置显示闪烁光标</span></div><button className={`switch ${settings.cursorBlink ? "on" : ""}`} role="switch" aria-checked={settings.cursorBlink} aria-label="光标闪烁" onClick={() => setSettings((s) => ({ ...s, cursorBlink: !s.cursorBlink }))}><span /></button></div>
      <div className="settings-section-label">键盘快捷键</div>
      <div className="shortcut-row"><span>新建终端</span><kbd>{info.platform === "darwin" ? "⌘" : "Ctrl"} T</kbd></div>
      <div className="shortcut-row"><span>关闭当前会话</span><kbd>{info.platform === "darwin" ? "⌘" : "Ctrl"} W</kbd></div>
      <div className="shortcut-row"><span>搜索终端输出</span><kbd>{info.platform === "darwin" ? "⌘" : "Ctrl"} F</kbd></div>
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

export function App() {
  const [info, setInfo] = useState(null);
  const [bootError, setBootError] = useState("");
  const [cwd, setCwd] = useState("");
  const [sessions, setSessions] = useState([]);
  const [activeId, setActiveId] = useState(null);
  const [focusedId, setFocusedId] = useState(null);
  const [agentSession, setAgentSession] = useState(null);
  const [agentVisible, setAgentVisible] = useState(true);
  const [agentStarting, setAgentStarting] = useState(false);
  const [profiles, setProfiles] = useState([]);
  const [profileMenu, setProfileMenu] = useState(null);
  const [dialog, setDialog] = useState(null);
  const [settingsOpen, setSettingsOpen] = useState(false);
  const [deleting, setDeleting] = useState(null);
  const [settings, setSettings] = useState(readSettings);
  const [toast, setToast] = useState(null);
  const [searchOpen, setSearchOpen] = useState(false);
  const [searchValue, setSearchValue] = useState("");
  const [searchFound, setSearchFound] = useState(true);
  const controls = useRef(new Map());
  const toastTimer = useRef(null);
  const searchInput = useRef(null);
  const started = useRef(false);
  const nextTerminal = useRef(1);
  const sessionsSnapshot = useRef(sessions);
  sessionsSnapshot.current = sessions;

  const notify = useCallback((message, type = "error") => {
    clearTimeout(toastTimer.current);
    setToast({ message, type });
    toastTimer.current = setTimeout(() => setToast(null), 6500);
  }, []);
  const register = useCallback((id, value) => {
    if (value) controls.current.set(id, value); else controls.current.delete(id);
  }, []);
  const onState = useCallback((id, status, exitCode) => {
    setSessions((previous) => previous.map((session) => session.id === id ? { ...session, status, exitCode } : session));
    setAgentSession((previous) => previous?.id === id ? { ...previous, status, exitCode } : previous);
  }, []);

  useEffect(() => {
    if (started.current) return;
    started.current = true;
    if (!api) { setBootError("无法连接桌面服务。请通过 omniTerm 桌面应用打开此窗口。"); return; }
    async function initialize() {
      try {
        const details = await api.getAppInfo();
        setInfo(details); setCwd(details.cwd || details.home); setProfiles(details.profiles || []);
        const session = await api.createSession({ kind: "shell", cwd: details.cwd || details.home });
        setSessions([{ ...session, title: "本地终端" }]); setActiveId(session.id); setFocusedId(session.id);
      } catch (error) { setBootError(error.message || String(error)); }
    }
    initialize();
    return () => clearTimeout(toastTimer.current);
  }, []);

  useEffect(() => {
    try { localStorage.setItem("omniterm-terminal-settings", JSON.stringify(settings)); }
    catch { notify("无法保存终端外观设置，本次会话中仍然生效。"); }
  }, [settings, notify]);

  const addSession = useCallback(async (kind = "shell", profile, directory) => {
    const session = await api.createSession({ kind, cwd: directory || cwd, ...(profile ? { profile } : {}) });
    const title = profile?.name || (kind === "shell" ? `本地终端 ${++nextTerminal.current}` : connectionLabels[kind] || session.title);
    setSessions((previous) => [...previous, { ...session, title }]);
    setActiveId(session.id); setFocusedId(session.id);
    return session;
  }, [cwd]);

  const closeSession = useCallback(async (id) => {
    try {
      await api.closeSession(id);
      if (agentSession?.id === id) { setAgentSession(null); setFocusedId(activeId); }
      else {
        const remaining = sessionsSnapshot.current.filter((session) => session.id !== id);
        setSessions((previous) => previous.filter((session) => session.id !== id));
        setActiveId((previous) => previous === id ? remaining.at(-1)?.id || null : previous);
        setFocusedId((previous) => previous === id ? remaining.at(-1)?.id || null : previous);
      }
    } catch (error) { notify(error.message); }
  }, [activeId, sessions, agentSession, notify]);

  const selectTab = (id) => { setActiveId(id); setFocusedId(id); requestAnimationFrame(() => controls.current.get(id)?.focus()); };
  const activeSession = sessions.find((session) => session.id === activeId);
  const selectedSession = focusedId === agentSession?.id && agentVisible ? agentSession : activeSession;
  const searchId = selectedSession?.id;
  const runControl = async (action, id = searchId) => {
    try { await controls.current.get(id)?.[action](); }
    catch (error) { notify(error.message); }
  };
  const showSearch = useCallback(() => { if (!selectedSession) return; setSearchOpen(true); requestAnimationFrame(() => searchInput.current?.focus()); }, [selectedSession]);
  const hideSearch = () => { setSearchOpen(false); controls.current.get(searchId)?.clearSearch(); controls.current.get(searchId)?.focus(); };
  const find = (value = searchValue, previous = false) => setSearchFound(controls.current.get(searchId)?.find(value, previous) ?? true);

  useEffect(() => {
    const handler = (event) => {
      if (dialog || settingsOpen || deleting || !info) return;
      if (!(event.metaKey || event.ctrlKey)) return;
      const key = event.key.toLowerCase();
      if (key === "t") { event.preventDefault(); addSession().catch((error) => notify(error.message)); }
      else if (key === "w" && selectedSession) { event.preventDefault(); closeSession(selectedSession.id); }
      else if (key === "f" && selectedSession) { event.preventDefault(); showSearch(); }
      else if (key === "tab" && sessions.length > 1) {
        event.preventDefault();
        const index = sessions.findIndex((session) => session.id === activeId);
        const next = sessions[(index + (event.shiftKey ? -1 : 1) + sessions.length) % sessions.length];
        setActiveId(next.id); setFocusedId(next.id); controls.current.get(next.id)?.focus();
      }
    };
    window.addEventListener("keydown", handler);
    return () => window.removeEventListener("keydown", handler);
  }, [dialog, settingsOpen, deleting, info, selectedSession, sessions, activeId, addSession, closeSession, showSearch, notify]);

  async function chooseDirectory() {
    try {
      const directory = await api.chooseDirectory();
      if (directory) { setCwd(directory); await addSession("shell", undefined, directory); }
    } catch (error) { notify(error.message); }
  }
  async function startAgent() {
    if (agentStarting) return;
    setAgentStarting(true);
    try {
      if (agentSession) { await api.closeSession(agentSession.id); setAgentSession(null); }
      const session = await api.createSession({ kind: "agent", cwd });
      setAgentSession({ ...session, title: "Codex Agent" }); setAgentVisible(true); setFocusedId(session.id);
    } catch (error) { notify(error.message); }
    finally { setAgentStarting(false); }
  }
  async function saveProfile(profile) { setProfiles(await api.saveProfile(profile)); }
  const openConnection = (kind, profile) => { setProfileMenu(null); setDialog({ kind, profile }); };
  const runningCount = sessions.filter((session) => !["exited", "error"].includes(session.status)).length + (agentSession && !["exited", "error"].includes(agentSession.status) ? 1 : 0);
  const modifier = info?.platform === "darwin" ? "⌘" : "Ctrl";

  if (!info) return <div className="app-boot"><div className="boot-logo"><TerminalWindow size={32} /></div><h1>omniTerm</h1>{bootError ? <><p className="boot-error">{bootError}</p><button className="button secondary" onClick={() => window.location.reload()}><ArrowClockwise size={16} />重新连接</button></> : <p><CircleNotch size={16} className="spin" />正在打开工作区…</p>}</div>;

  return <div className={`app-shell platform-${info.platform}`}>
    <header className="titlebar">
      <div className="window-brand"><TerminalWindow size={19} weight="duotone" /><strong>omniTerm</strong></div>
      <div className="window-title"><FolderSimple size={13} /><span>{basename(cwd)}</span><span className="title-divider">/</span><span>工作区</span></div>
      <div className="titlebar-actions"><button className={`titlebar-button ${agentVisible ? "active" : ""}`} title={agentVisible ? "隐藏 Agent 面板（会话继续运行）" : "显示 Agent 面板"} onClick={() => setAgentVisible((visible) => !visible)}><Robot size={16} /><span>Agent</span><SidebarSimple size={15} /></button></div>
    </header>

    <div className="workspace-layout">
      <aside className="sidebar">
        <div className="sidebar-section-heading"><span>工作区</span><IconButton label="打开工作文件夹" onClick={chooseDirectory}><FolderOpen size={16} /></IconButton></div>
        <button className="workspace-switcher" onClick={chooseDirectory} title={cwd}><span className="workspace-avatar"><FolderSimple size={20} /></span><span><strong>{basename(cwd)}</strong><small>{pathLabel(cwd, info.home)}</small></span><CaretRight size={13} /></button>
        <nav className="workspace-nav" aria-label="工作区导航">
          <button className={`nav-item ${selectedSession?.kind === "shell" ? "active" : ""}`} onClick={() => { const local = sessions.find((session) => session.kind === "shell"); if (local) selectTab(local.id); else addSession().catch((error) => notify(error.message)); }}><TerminalWindow size={18} /><span>本地终端</span><span className="nav-count">{sessions.filter((session) => session.kind === "shell").length}</span></button>
          <button className={`nav-item ${focusedId === agentSession?.id && agentVisible ? "active" : ""}`} onClick={() => { setAgentVisible(true); if (agentSession) { setFocusedId(agentSession.id); controls.current.get(agentSession.id)?.focus(); } }}><Robot size={18} /><span>Agent</span>{agentSession?.status === "running" ? <span className="status-dot" /> : <span className="nav-meta">Codex</span>}</button>
        </nav>
        <div className="sidebar-section-heading saved-heading"><span>已保存的连接</span><IconButton label="添加连接" onClick={() => openConnection("ssh")}><Plus size={15} /></IconButton></div>
        <div className="saved-connections">
          {profiles.length ? profiles.map((profile) => {
            const Icon = kindIcons[profile.kind] || HardDrives;
            return <div className="profile-wrap" key={profile.id}><div className="profile-row"><button className="profile-connect" onClick={() => addSession(profile.kind, profile).catch((error) => notify(error.message))} title={`连接 ${profile.name}`}><Icon size={17} /><span><strong>{profile.name}</strong><small>{profile.kind === "serial" ? profile.device : `${profile.username ? `${profile.username}@` : ""}${profile.host}`}</small></span></button><IconButton label={`${profile.name}的选项`} onClick={() => setProfileMenu(profileMenu === profile.id ? null : profile.id)}><DotsThree size={19} /></IconButton></div>{profileMenu === profile.id && <div className="profile-menu"><button onClick={() => openConnection(profile.kind, profile)}><PencilSimple size={14} />编辑连接</button><button onClick={() => { setProfileMenu(null); setDeleting(profile); }}><Trash size={14} />删除连接</button></div>}</div>;
          }) : <div className="connections-empty"><HardDrives size={23} /><p>你的连接，会在这里</p><span>保存常用主机，一键连接。</span><button className="text-button" onClick={() => openConnection("ssh")}><Plus size={13} />添加第一个连接</button></div>}
        </div>
        <div className="sidebar-bottom"><div className="sidebar-section-heading"><span>连接工具</span></div><div className="connection-tools">{Object.entries(connectionLabels).map(([kind, label]) => { const Icon = kindIcons[kind]; return <button key={kind} onClick={() => openConnection(kind)}><Icon size={16} /><span>{label}</span></button>; })}</div><button className="settings-button" onClick={() => setSettingsOpen(true)}><GearSix size={18} /><span>设置</span><span className="version-label">v{info.version}</span></button></div>
      </aside>

      <main className="main-workspace">
        <div className="workspace-toolbar"><div className="breadcrumb" title={activeSession?.cwd || cwd}><FolderSimple size={16} /><span>工作区</span><CaretRight size={12} /><strong>{pathLabel(activeSession?.cwd || cwd, info.home)}</strong></div><button className="button toolbar-button" onClick={chooseDirectory}><FolderOpen size={15} />打开文件夹</button></div>
        <div className={`panes ${agentVisible ? "with-agent" : ""}`}>
          <section className="terminal-pane" aria-label="终端工作区">
            <div className="tabstrip"><div className="tabs" role="tablist" aria-label="终端会话">{sessions.map((session) => { const Icon = kindIcons[session.kind] || TerminalWindow; return <div key={session.id} className={`tab ${activeId === session.id ? "active" : ""}`}><button role="tab" aria-selected={activeId === session.id} onClick={() => selectTab(session.id)}><Icon size={15} /><span>{session.title}</span>{["exited", "error"].includes(session.status) && <Circle size={6} weight="fill" className="inactive-indicator" />}</button><IconButton label={`关闭 ${session.title}`} onClick={() => closeSession(session.id)}><X size={12} /></IconButton></div>; })}</div><IconButton className="new-tab" label={`新建终端（${modifier}+T）`} onClick={() => addSession().catch((error) => notify(error.message))}><Plus size={17} /></IconButton></div>
            <div className="terminal-toolbar"><div className="session-detail">{activeSession ? <><span className={`status-dot ${activeSession.status === "running" ? "" : "muted"}`} /><span>{activeSession.kind === "shell" ? "本地 Shell" : connectionLabels[activeSession.kind]}</span><span className="toolbar-dot">·</span><span>{activeSession.status === "exited" ? "会话已结束" : activeSession.status === "error" ? "启动失败" : activeSession.status === "running" ? "已连接" : "正在连接"}</span></> : <span>终端</span>}</div><div className="terminal-actions"><IconButton label={`搜索终端（${modifier}+F）`} onClick={showSearch} disabled={!selectedSession}><MagnifyingGlass size={15} /></IconButton><IconButton label="复制选中文本" onClick={() => runControl("copy")} disabled={!selectedSession}><Copy size={15} /></IconButton><IconButton label="粘贴到终端" onClick={() => runControl("paste")} disabled={!selectedSession}><ClipboardText size={15} /></IconButton><IconButton label="清空终端屏幕" onClick={() => runControl("clear")} disabled={!selectedSession}><Trash size={15} /></IconButton></div></div>
            {searchOpen && <div className="search-bar"><MagnifyingGlass size={15} /><input ref={searchInput} value={searchValue} placeholder={`搜索${selectedSession?.kind === "agent" ? " Agent" : "终端"}输出`} onChange={(event) => { setSearchValue(event.target.value); find(event.target.value); }} onKeyDown={(event) => { if (event.key === "Enter") { event.preventDefault(); find(searchValue, event.shiftKey); } if (event.key === "Escape") hideSearch(); }} /><span className="search-feedback">{searchValue && !searchFound ? "未找到" : ""}</span><IconButton label="上一个结果" onClick={() => find(searchValue, true)}><ArrowUp size={14} /></IconButton><IconButton label="下一个结果" onClick={() => find()}><ArrowDown size={14} /></IconButton><IconButton label="关闭搜索" onClick={hideSearch}><X size={14} /></IconButton></div>}
            <div className="terminal-stack">{sessions.map((session) => <TerminalView key={session.id} session={session} visible={activeId === session.id} settings={settings} api={api} register={register} onState={onState} onError={notify} onFocus={setFocusedId} />)}{!sessions.length && <div className="terminal-empty"><div className="empty-terminal-icon"><TerminalWindow size={34} weight="light" /></div><h2>开始一个新的会话</h2><p>打开本地终端，或连接到你的远程主机。</p><button className="button primary" onClick={() => addSession().catch((error) => notify(error.message))}><Plus size={16} />新建终端<kbd>{modifier} T</kbd></button></div>}</div>
            <div className="terminal-footnote"><span>{activeSession ? pathLabel(activeSession.cwd, info.home) : pathLabel(cwd, info.home)}</span><span>UTF-8</span></div>
          </section>

          <aside className={`agent-pane ${agentVisible ? "is-visible" : ""}`} aria-label="Agent 面板">
            <div className="agent-heading"><Robot size={18} /><strong>Agent</strong><span className="agent-model-label">Codex</span><div className="agent-heading-actions">{agentSession && <IconButton label="结束 Agent 会话" onClick={() => closeSession(agentSession.id)}><Trash size={15} /></IconButton>}<IconButton label="隐藏 Agent 面板" onClick={() => setAgentVisible(false)}><SidebarSimple size={17} /></IconButton></div></div>
            <div className="agent-context"><FolderSimple size={14} /><span title={agentSession?.cwd || cwd}>{basename(agentSession?.cwd || cwd)}</span><span className="context-local">本地工作区</span></div>
            {agentSession ? <div className="agent-terminal-stack"><TerminalView session={agentSession} visible={agentVisible} settings={settings} api={api} register={register} onState={onState} onError={notify} onFocus={setFocusedId} />{["exited", "error"].includes(agentSession.status) && <div className="agent-restart"><button className="button secondary" onClick={startAgent} disabled={agentStarting}><ArrowClockwise size={15} />重新启动 Agent</button></div>}</div> : <div className="agent-invitation"><div className="agent-symbol"><Robot size={32} weight="light" /><span className={`agent-availability ${info.agent?.available ? "available" : ""}`} /></div><span className="agent-eyebrow">你的终端搭档</span><h2>从一个想法开始。</h2><p>让 Agent 理解代码、处理任务。<br />你掌握每一步的决定权。</p><button className="button primary launch-agent" onClick={startAgent} disabled={!info.agent?.available || agentStarting}>{agentStarting ? <CircleNotch className="spin" size={17} /> : <Robot size={17} />}启动 Agent<ArrowRight size={15} /></button><div className="agent-local-note"><Cpu size={13} /><span>{info.agent?.available ? "Codex 运行于本机" : "本地 Agent 暂不可用"}</span></div>{!info.agent?.available && <div className="agent-unavailable"><Info size={15} /><span>{info.agent?.error || "未找到 Agent 程序，请完成本地 Agent 安装。"}</span></div>}<div className="agent-explanation"><div><Code size={17} /><span>理解当前项目与终端环境</span></div><div><TerminalWindow size={17} /><span>编写代码，执行经你批准的操作</span></div><div><PlugsConnected size={17} /><span>在同一个工作区中完成任务</span></div></div></div>}
            <div className="agent-footer"><span className={`status-dot ${info.agent?.available ? "" : "muted"}`} /><span>{agentSession?.status === "running" ? "Agent 会话运行中" : "由本地 Codex 提供支持"}</span><span className="agent-footer-label">Agent 模式</span></div>
          </aside>
        </div>
      </main>
    </div>
    <footer className="statusbar"><div><Monitor size={13} /><span>{platformNames[info.platform] || info.platform}</span><span className="statusbar-separator" /><span className="status-dot" /><span>{runningCount} 个活动会话</span></div><div><span className={`status-dot ${info.agent?.available ? "" : "muted"}`} /><span>本地 Agent {info.agent?.available ? "已就绪" : "未就绪"}</span><span className="statusbar-separator" /><button onClick={() => setSettingsOpen(true)} title="终端设置"><Command size={12} /><span>快捷键</span></button></div></footer>
    {dialog && <ConnectionDialog initial={dialog} onClose={() => setDialog(null)} onSave={saveProfile} onConnect={addSession} />}
    {settingsOpen && <SettingsDialog settings={settings} setSettings={setSettings} info={info} onClose={() => setSettingsOpen(false)} />}
    {deleting && <DeleteDialog profile={deleting} onClose={() => setDeleting(null)} onConfirm={async (id) => setProfiles(await api.deleteProfile(id))} />}
    {toast && <div className={`toast ${toast.type}`} role="alert">{toast.type === "error" ? <WarningCircle size={19} /> : <Check size={19} />}<span>{toast.message}</span><IconButton label="关闭提示" onClick={() => setToast(null)}><X size={15} /></IconButton></div>}
    {bootError && info && <div className="toast error" role="alert"><WarningCircle size={19} /><span>{bootError}</span><IconButton label="关闭启动提示" onClick={() => setBootError("")}><X size={15} /></IconButton></div>}
  </div>;
}
