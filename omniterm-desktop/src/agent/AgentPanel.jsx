import { forwardRef, useCallback, useEffect, useImperativeHandle, useMemo, useReducer, useRef, useState } from "react";
import { Broom, Check, CircleNotch, Cpu, Crosshair, GearSix, HardDrives, Pause, PaperPlaneTilt, Play, Robot, ShieldCheck, SidebarSimple, Stack, Stop, TerminalWindow, WarningCircle, X } from "@phosphor-icons/react";
import { ProviderDialog } from "./ProviderDialog.jsx";
import { INTERACTION_LABELS, OPEN_OPERATIONS, OPERATION_LABELS, WIRE_LABELS, citationParts, conversationReducer, emptyConversation, executable, mentionMatches, mentionQuery, pruneSelection, refsFor, scopeSummary, terminalName } from "./agent-logic.js";

const POLL_MS = 1500;
const PROFILE_KEY = "omniterm-agent-profile";
const GRANT_MS = 10 * 60 * 1000;

function Citations({ text, terminals, onLocate }) {
  return <>{citationParts(text, terminals).map((part, index) => typeof part === "string" ? <span key={index}>{part}</span> : <button key={index} className="citation" title={terminalName(part.terminal)} onClick={() => onLocate(part.terminal)}>{part.text}</button>)}</>;
}

function OperationCard({ entry, terminals, api, notify }) {
  const { operation } = entry;
  const target = terminals.find((terminal) => terminal.ref.sessionId === operation.target.sessionId);
  const name = target ? terminalName(target) : `${operation.target.sessionId.slice(0, 8)}（已关闭）`;
  const pending = operation.status === "pendingApproval";
  const interaction = target?.interaction?.state ?? "unknown";
  const canRun = interaction === "ready";
  const act = (task) => Promise.resolve().then(task).catch((error) => notify(error.message));
  const approve = (scope, mode) => act(() => api.approveOperation({ operationId: operation.operationId, scope, mode, ...(scope === "taskTarget" ? { ttlMs: GRANT_MS } : {}) }));
  return <div className={`operation-card status-${operation.status}`}>
    <div className="operation-target"><Crosshair size={14} /><span>执行于 <strong>{name}</strong></span><span className="operation-status">{OPERATION_LABELS[operation.status] ?? operation.status}</span></div>
    {operation.rationale && <p>{operation.rationale}</p>}
    <pre>{operation.command}</pre>
    {operation.reason && <p className="operation-reason">{operation.reason}</p>}
    {operation.exitCode !== null && operation.exitCode !== undefined && <p className="operation-reason">退出码 {operation.exitCode}</p>}
    {pending && <>
      <div className="operation-state">终端状态：{INTERACTION_LABELS[interaction] ?? interaction}{!canRun && "。无法确认空闲，只能输入后由你手动回车。"}</div>
      <div className="operation-actions">
        {canRun && <button className="button primary" onClick={() => approve("once", "run")}><Check size={14} />批准执行</button>}
        {canRun && <button className="button secondary" title="10 分钟内，本任务发往这个终端的命令不再询问" onClick={() => approve("taskTarget", "run")}><ShieldCheck size={14} />对此目标放行 10 分钟</button>}
        <button className={`button ${canRun ? "ghost" : "primary"}`} onClick={() => approve("once", "stage")}>输入但不回车</button>
        <button className="button ghost" onClick={() => act(() => api.denyOperation(operation.operationId))}><X size={14} />拒绝</button>
      </div>
    </>}
    {!pending && OPEN_OPERATIONS.has(operation.status) && <div className="operation-actions"><button className="button ghost" onClick={() => act(() => api.cancelOperation(operation.operationId))}><Stop size={14} />{operation.status === "running" ? "中断（Ctrl+C）" : "取消"}</button></div>}
  </div>;
}

function Entry({ entry, terminals, api, notify, onLocate }) {
  switch (entry.kind) {
    case "user": return <div className="agent-message user">{entry.text}</div>;
    case "assistant": return <div className="agent-message assistant"><Citations text={entry.text} terminals={terminals} onLocate={onLocate} /></div>;
    case "reasoning": return <details className="agent-reasoning"><summary>思考过程</summary><p>{entry.text}</p></details>;
    case "tool": return <div className={`agent-tool ${entry.success === false ? "failed" : ""}`}>{entry.status === "inProgress" ? <CircleNotch className="spin" size={12} /> : entry.success === false ? <WarningCircle size={12} /> : <Check size={12} />}<span>{entry.tool}</span></div>;
    case "context": return <div className="agent-shared"><Stack size={13} /><span>已向 {entry.provider} · {entry.model} 共享：{entry.terminals.map((terminal) => `${terminal.label} · ${terminal.ref.sessionId.slice(0, 8)}${terminal.truncated ? `（省略 ${terminal.omittedLines} 行）` : ""}`).join("，") || "无"}{entry.unavailable.length ? `；${entry.unavailable.length} 个终端已关闭或重连，未共享` : ""}</span></div>;
    case "operation": return <OperationCard entry={entry} terminals={terminals} api={api} notify={notify} />;
    case "error": return <div className="agent-error"><WarningCircle size={15} /><span>{entry.text}</span></div>;
    case "notice": return <div className="agent-notice">{entry.text}</div>;
    default: return null;
  }
}

function TerminalPicker({ terminals, context, targets, onContext, onTargets, onAllContext, onStage, platform }) {
  return <div className="terminal-picker">
    <div className="picker-header"><span>终端</span><span>上下文</span><span>执行</span></div>
    {terminals.map((terminal) => {
      const id = terminal.ref.sessionId;
      const Icon = terminal.kind === "shell" ? TerminalWindow : HardDrives;
      return <div className="picker-row" key={id}>
        <div className="picker-name" title={`${terminal.tab ? `标签 ${terminal.tab.index}${terminal.pane ? ` · 窗格 ${terminal.pane}` : ""}` : "位置未知"} · ${terminal.shortId}`}><Icon size={14} /><span><strong>{terminal.label}</strong><small>{terminal.connection.configured.host ?? (terminal.cwd.value || "目录未知")} · {INTERACTION_LABELS[terminal.interaction.state] ?? "未知"}{terminal.focused ? " · 当前焦点" : ""}</small></span>{terminal.interaction.state === "unknown" && (terminal.kind === "ssh" || (terminal.kind === "shell" && platform !== "win32")) && <button className="picker-link" title="向该终端输入一段启用就绪标记的脚本（zsh 或 bash 4.4+），由你按回车确认。启用后，已批准的命令才能自动提交。" onClick={() => onStage(terminal)}>启用就绪标记</button>}</div>
        <input type="checkbox" aria-label={`将 ${terminal.label} 加入上下文`} checked={context.has(id)} onChange={() => onContext(id)} />
        <input type="radio" name="agent-target" aria-label={`将 ${terminal.label} 设为执行目标`} disabled={!executable(terminal)} checked={targets.has(id)} onChange={() => onTargets(id)} />
      </div>;
    })}
    <div className="picker-footer"><button className="picker-link" onClick={onAllContext}>选择全部当前终端</button><button className="picker-link" disabled={!targets.size} onClick={() => onTargets(null)}>只读（无执行目标）</button></div>
  </div>;
}

export const AgentPanel = forwardRef(function AgentPanel({ api, info, visible, sessionsVersion, focusedId, getSelection, onActivate, onHide, notify }, ref) {
  const [conversation, dispatch] = useReducer(conversationReducer, emptyConversation);
  const [terminals, setTerminals] = useState([]);
  const [context, setContext] = useState(new Set());
  const [targets, setTargets] = useState(new Set());
  const [attached, setAttached] = useState(new Map());
  const [providers, setProviders] = useState(null);
  const [profileId, setProfileId] = useState(() => localStorage.getItem(PROFILE_KEY));
  const [activeProfile, setActiveProfile] = useState(null);
  const [settingsOpen, setSettingsOpen] = useState(false);
  const [pickerOpen, setPickerOpen] = useState(false);
  const [paused, setPaused] = useState(false);
  const [text, setText] = useState("");
  const [sending, setSending] = useState(false);
  const log = useRef(null);
  const contextTouched = useRef(false);
  const counter = useRef(0);

  const profile = providers?.profiles.find((item) => item.id === profileId) ?? providers?.profiles[0] ?? null;
  const refresh = useCallback(() => api.listWorkspace().then((list) => setTerminals(list.terminals)).catch(() => {}), [api]);

  useEffect(() => { api.providerState().then(setProviders).catch((error) => notify(error.message)); }, [api, notify]);
  useEffect(() => api.onAgentEvent(dispatch), [api]);
  useEffect(() => {
    if (!visible) return undefined;
    refresh();
    const timer = setInterval(refresh, POLL_MS);
    return () => clearInterval(timer);
  }, [visible, refresh, sessionsVersion]);
  useEffect(() => {
    setContext((ids) => pruneSelection(ids, terminals));
    setTargets((ids) => pruneSelection(ids, terminals));
    setAttached((map) => { const live = new Set(terminals.map((terminal) => terminal.ref.sessionId)); return [...map.keys()].every((id) => live.has(id)) ? map : new Map([...map].filter(([id]) => live.has(id))); });
  }, [terminals]);
  // Until the user chooses a scope, it follows the focused terminal. Execution never does.
  useEffect(() => {
    if (!contextTouched.current && focusedId && terminals.some((terminal) => terminal.ref.sessionId === focusedId)) setContext(new Set([focusedId]));
  }, [focusedId, terminals]);
  useEffect(() => { log.current?.scrollTo({ top: log.current.scrollHeight }); }, [conversation.entries]);
  useEffect(() => { if (profile) localStorage.setItem(PROFILE_KEY, profile.id); }, [profile]);

  const toggleContext = (id) => {
    contextTouched.current = true;
    setContext((ids) => { const next = new Set(ids); if (!next.delete(id)) next.add(id); return next; });
  };
  const selectTarget = (id) => setTargets(id ? new Set([id]) : new Set());

  useImperativeHandle(ref, () => ({
    attachSelection(id, selection) {
      contextTouched.current = true;
      setContext((ids) => new Set(ids).add(id));
      setAttached((map) => new Map(map).set(id, selection));
    },
  }), []);

  const mention = mentionQuery(text);
  const mentions = mention === null ? [] : mentionMatches(terminals, mention);
  const pickMention = (terminal) => {
    toggleContextOn(terminal.ref.sessionId);
    setText((value) => value.replace(/@[^\s@]*$/, `@${terminal.shortId} `));
  };
  const toggleContextOn = (id) => { contextTouched.current = true; setContext((ids) => new Set(ids).add(id)); };

  async function send() {
    const message = text.trim();
    if (!message || sending || conversation.running || !profile) return;
    setSending(true);
    try {
      const list = await api.listWorkspace();
      setTerminals(list.terminals);
      const selections = {};
      for (const id of context) {
        const selection = attached.get(id) ?? getSelection(id);
        if (selection?.trim()) selections[id] = selection.trim();
      }
      // A different provider means a new engine thread; nothing earlier is resent to it.
      if (activeProfile && activeProfile !== profile.id) dispatch({ type: "notice", text: `已切换到 ${profile.name} · ${profile.model}。这是一段新对话，此前内容不会发送给新的模型服务。` });
      dispatch({ type: "user", id: `user-${++counter.current}`, text: message });
      setText(""); setAttached(new Map());
      // Pausing outlives a message: only the user's resume shares terminals again.
      await api.agentSend({ profileId: profile.id, text: message, paused, context: paused ? [] : refsFor(list.terminals, context), targets: refsFor(list.terminals, targets), selections: paused ? {} : selections, cwd: info.cwd });
      setActiveProfile(profile.id);
    } catch (error) {
      dispatch({ type: "error", message: error.message });
    } finally { setSending(false); }
  }
  const scopeRefs = () => ({ context: refsFor(terminals, context), targets: refsFor(terminals, targets) });
  const togglePause = async () => {
    const next = !paused;
    setPaused(next);
    try {
      await api.agentPause({ paused: next, ...scopeRefs() });
      dispatch({ type: "notice", text: next ? "已暂停共享：不再采集新的终端内容。已发送的内容仍在当前对话中；要排除它们，请新建对话。" : "已恢复共享。" });
    } catch (error) { notify(error.message); }
  };
  const newConversation = async () => {
    try { await api.agentReset(); dispatch({ type: "clear" }); setActiveProfile(null); setPaused(false); }
    catch (error) { notify(error.message); }
  };
  const stageIntegration = (terminal) => api.stageShellIntegration(terminal.ref.sessionId).then(() => { onActivate(terminal.ref.sessionId); notify("脚本已输入到该终端，确认后按回车启用。", "success"); }).catch((error) => notify(error.message));

  const empty = !conversation.entries.length;
  const available = info.agent?.available;
  const targetName = useMemo(() => targets.size ? scopeSummary(terminals, targets) : "只读", [terminals, targets]);
  const contextName = useMemo(() => paused ? "已暂停" : scopeSummary(terminals, context), [terminals, context, paused]);
  return <aside className={`agent-pane ${visible ? "is-visible" : ""}`} aria-label="omniTerm Agent 面板">
    <div className="agent-heading"><Robot size={18} /><strong>omniTerm Agent</strong><div className="agent-heading-actions">
      <button type="button" className="icon-button" title={paused ? "恢复共享" : "暂停共享"} aria-label={paused ? "恢复共享" : "暂停共享"} onClick={togglePause}>{paused ? <Play size={15} /> : <Pause size={15} />}</button>
      <button type="button" className="icon-button" title="新建对话（清除此前共享的内容）" aria-label="新建对话" onClick={newConversation}><Broom size={15} /></button>
      <button type="button" className="icon-button" title="隐藏 Agent 面板" aria-label="隐藏 Agent 面板" onClick={onHide}><SidebarSimple size={17} /></button>
    </div></div>
    <div className="agent-scope">
      <button className="scope-chip" aria-expanded={pickerOpen} onClick={() => setPickerOpen(!pickerOpen)} title="选择共享给模型的终端"><Stack size={13} /><span><small>上下文</small>{contextName}</span></button>
      <button className={`scope-chip ${targets.size ? "armed" : ""}`} aria-expanded={pickerOpen} onClick={() => setPickerOpen(!pickerOpen)} title="选择允许提议命令的终端；切换焦点不会改变它"><Crosshair size={13} /><span><small>执行目标</small>{targetName}</span></button>
      <button className="scope-chip" onClick={() => setSettingsOpen(true)} title="模型设置"><Cpu size={13} /><span><small>{profile ? (WIRE_LABELS[profile.wireApi] ?? profile.wireApi) : "模型"}</small>{profile ? `${profile.name} · ${profile.model}${profile.chatOnly ? " · 仅聊天" : ""}` : "未配置"}</span></button>
    </div>
    {pickerOpen && <TerminalPicker terminals={terminals} context={context} targets={targets} onContext={toggleContext} onTargets={selectTarget} onAllContext={() => { contextTouched.current = true; setContext(new Set(terminals.map((terminal) => terminal.ref.sessionId))); }} onStage={stageIntegration} platform={info.platform} />}
    <div className="agent-log" ref={log}>
      {empty && <div className="agent-invitation"><div className="agent-symbol"><Robot size={32} weight="light" /></div><span className="agent-eyebrow">你的终端搭档</span><h2>从一个问题开始。</h2><p>选择要共享的终端和执行目标。<br />每条命令都由你批准，只在你选定的终端运行。</p>{!available && <div className="agent-unavailable"><WarningCircle size={15} /><span>{info.agent?.error || "未找到 Agent 引擎。"}</span></div>}{available && providers && !providers.profiles.length && <button className="button primary" onClick={() => setSettingsOpen(true)}><GearSix size={16} />配置模型服务</button>}</div>}
      {conversation.entries.map((entry) => <Entry key={`${entry.kind}-${entry.id}`} entry={entry} terminals={terminals} api={api} notify={notify} onLocate={(terminal) => onActivate(terminal.ref.sessionId)} />)}
      {conversation.running && <div className="agent-working"><CircleNotch className="spin" size={13} /><span>{conversation.retrying ? "正在重试请求…" : "omniTerm Agent 正在处理…"}</span></div>}
    </div>
    <div className="agent-composer">
      {attached.size > 0 && <div className="attached-note"><Stack size={12} /><span>已附加 {attached.size} 段终端选区</span><button aria-label="移除附加的选区" onClick={() => setAttached(new Map())}><X size={12} /></button></div>}
      {mentions.length > 0 && <div className="mention-menu" role="listbox">{mentions.map((terminal) => <button key={terminal.ref.sessionId} role="option" aria-selected="false" onClick={() => pickMention(terminal)}>{terminalName(terminal)}</button>)}</div>}
      <textarea rows={3} value={text} placeholder={profile ? "描述问题，输入 @ 引用终端…" : "请先配置模型服务"} disabled={!profile || !available} onChange={(event) => setText(event.target.value)} onKeyDown={(event) => { if (event.key === "Enter" && !event.shiftKey && !event.nativeEvent.isComposing) { event.preventDefault(); send(); } }} />
      <div className="composer-actions"><span>{paused ? "共享已暂停" : `将共享 ${context.size} 个终端${targets.size ? " · 可提议命令" : " · 只读"}`}</span>{conversation.running ? <button className="button secondary" onClick={() => api.agentCancel().catch((error) => notify(error.message))}><Stop size={14} />停止</button> : <button className="button primary" disabled={!text.trim() || !profile || sending} onClick={send}>{sending ? <CircleNotch className="spin" size={14} /> : <PaperPlaneTilt size={14} />}发送</button>}</div>
    </div>
    {settingsOpen && providers && <ProviderDialog api={api} state={providers} notify={notify} onState={(next) => { setProviders(next); }} onClose={(id) => { setSettingsOpen(false); if (id) setProfileId(id); }} />}
  </aside>;
});
