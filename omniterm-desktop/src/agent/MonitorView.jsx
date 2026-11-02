import { useEffect, useMemo, useState } from "react";
import { Check, Pause, Play, Plus, Stop, Trash, X } from "@phosphor-icons/react";
import { MONITOR_STATUS, buildMonitorRequest, emptyMonitorForm, monitorMetrics, monitorReducer, sparkline, targetSummary } from "./monitor-logic.js";

const FIELDS = { process: ["cpu", "memoryKb", "threads"], thread: ["cpu"], log: ["text"], sharedMemory: [] };

function MonitorForm({ api, terminals, notify, onDone }) {
  const [form, setForm] = useState(emptyMonitorForm);
  const update = (key) => (event) => setForm((previous) => ({ ...previous, [key]: event.target.type === "checkbox" ? event.target.checked : event.target.value }));
  const hosts = terminals.filter(({ kind }) => kind === "ssh");
  const targets = terminals.filter(({ kind, status }) => ["shell", "ssh"].includes(kind) && status === "running");
  async function create() {
    try { await api.monitorCreate(buildMonitorRequest(form, terminals)); onDone(); }
    catch (error) { notify(error.message); }
  }
  const fields = form.kind === "sharedMemory" ? form.layout.split("\n").map((line) => line.split(":")[0].trim()).filter(Boolean) : FIELDS[form.kind];
  return <div className="monitor-form">
    <div className="monitor-row">
      <label>主机<select value={form.host} onChange={update("host")}><option value="local">本机</option>{hosts.map((terminal) => <option key={terminal.ref.sessionId} value={terminal.ref.sessionId}>{terminal.label}</option>)}</select></label>
      <label>监控对象<select value={form.kind} onChange={update("kind")}><option value="process">进程</option><option value="thread">线程（Linux）</option><option value="log">日志文件</option><option value="sharedMemory">共享内存（只读）</option></select></label>
    </div>
    {form.kind === "process" && <label>进程 ID 或进程名<input value={form.process} onChange={update("process")} /></label>}
    {form.kind === "thread" && <div className="monitor-row"><label>进程 ID<input value={form.pid} onChange={update("pid")} /></label><label>线程 ID<input value={form.tid} onChange={update("tid")} /></label></div>}
    {form.kind === "log" && <label>日志路径<input value={form.path} placeholder="/var/log/app.log" onChange={update("path")} /></label>}
    {form.kind === "sharedMemory" && <>
      <label>区域名称（/dev/shm 下）<input value={form.region} onChange={update("region")} /></label>
      <label>布局（每行：字段:偏移:类型，例如 count:0:u32）<textarea rows={3} value={form.layout} onChange={update("layout")} /></label>
    </>}
    <div className="monitor-row">
      <label>条件字段<select value={form.field} onChange={update("field")}><option value="">不设置</option>{fields.map((field) => <option key={field} value={field}>{field}</option>)}<option value="exists">目标消失</option></select></label>
      {form.field && form.field !== "exists" && <label>{form.field === "text" ? "包含文本" : "大于"}<input value={form.value} onChange={update("value")} /></label>}
    </div>
    <div className="monitor-row"><label>采样间隔（秒）<input type="number" min="1" value={form.intervalSeconds} onChange={update("intervalSeconds")} /></label><label>时长（分钟）<input type="number" min="1" value={form.durationMinutes} onChange={update("durationMinutes")} /></label></div>
    <label className="provider-check"><input type="checkbox" checked={form.withAction} onChange={update("withAction")} />条件满足时自动执行一条命令</label>
    {form.withAction && <>
      <label>命令（固定不变，Agent 不能修改）<input value={form.command} onChange={update("command")} /></label>
      <div className="monitor-row">
        <label>目标终端<select value={form.terminal} onChange={update("terminal")}><option value="">请选择</option>{targets.map((terminal) => <option key={terminal.ref.sessionId} value={terminal.ref.sessionId}>{terminal.label} · {terminal.shortId}</option>)}</select></label>
        <label>最多执行次数<input type="number" min="1" value={form.maxRuns} onChange={update("maxRuns")} /></label>
      </div>
      <p className="monitor-note">批准后，在设定时长内，这条命令可以不经确认地执行。它仍要通过就绪检查；未开启“自动执行”的 SSH 终端不会执行。</p>
    </>}
    <div className="operation-actions"><button className="button primary" onClick={create}><Plus size={14} />创建</button><button className="button ghost" onClick={onDone}>取消</button></div>
  </div>;
}

function MonitorCard({ monitor, api, notify }) {
  const act = (task) => Promise.resolve().then(task).catch((error) => notify(error.message));
  const live = ["running", "paused"].includes(monitor.status);
  const metrics = monitorMetrics(monitor.lastSample);
  const series = useMemo(() => sparkline(monitor.series, metrics[0]?.[0]), [monitor.series, metrics]);
  return <div className={`monitor-card status-${monitor.status}`}>
    <div className="monitor-title"><strong>{monitor.label || targetSummary(monitor.target)}</strong><span className="monitor-status">{MONITOR_STATUS[monitor.status] ?? monitor.status}</span></div>
    <div className="monitor-note">{monitor.host === "local" ? "本机" : "SSH"} · {targetSummary(monitor.target)} · 每 {monitor.intervalSeconds} 秒{monitor.proposedBy === "agent" ? " · Agent 提议" : ""}</div>
    {monitor.conditions.map((condition, index) => <div className="monitor-note" key={index}>条件：{condition.op === "gone" ? "目标消失" : `${condition.field} ${condition.op} ${condition.value}`}</div>)}
    {monitor.action && <><div className="monitor-note">{monitor.status === "pending" ? "批准后可自动执行：" : "自动操作："}（最多 {monitor.action.maxRuns} 次，已执行 {monitor.actionRuns} 次）</div><div className="monitor-action">{monitor.action.command}</div></>}
    {metrics.length > 0 && <div className="monitor-values">{metrics.map(([key, value]) => `${key} ${value}`).join(" · ")}</div>}
    {series && <svg className="monitor-sparkline" viewBox="0 0 100 26" preserveAspectRatio="none" role="img" aria-label="趋势"><polyline fill="none" stroke="currentColor" strokeWidth="1" vectorEffect="non-scaling-stroke" points={series} /></svg>}
    <details><summary className="monitor-note">操作记录（{monitor.history.length}）</summary><ul className="monitor-history">{monitor.history.slice(-12).reverse().map((entry, index) => <li key={index}>{new Date(entry.at).toLocaleTimeString()} {entry.message}{entry.reason ? `（${entry.reason}）` : ""}</li>)}</ul></details>
    <div className="operation-actions">
      {monitor.status === "pending" && <><button className="button primary" onClick={() => act(() => api.monitorApprove(monitor.id))}><Check size={14} />批准</button><button className="button ghost" onClick={() => act(() => api.monitorDeny(monitor.id))}><X size={14} />拒绝</button></>}
      {monitor.status === "running" && <button className="button ghost" onClick={() => act(() => api.monitorPause(monitor.id))}><Pause size={14} />暂停</button>}
      {monitor.status === "paused" && <button className="button ghost" onClick={() => act(() => api.monitorResume(monitor.id))}><Play size={14} />继续</button>}
      {live && monitor.action && <button className="button ghost" onClick={() => act(() => api.monitorRevokeAction(monitor.id))}>撤销自动操作</button>}
      {live && <button className="button ghost" onClick={() => act(() => api.monitorStop(monitor.id))}><Stop size={14} />停止</button>}
      {!live && monitor.status !== "pending" && <button className="button ghost" onClick={() => act(() => api.monitorRemove(monitor.id))}><Trash size={14} />删除</button>}
    </div>
  </div>;
}

export function MonitorView({ api, terminals, notify }) {
  const [monitors, setMonitors] = useState([]);
  const [creating, setCreating] = useState(false);
  useEffect(() => {
    api.monitorList().then(setMonitors).catch((error) => notify(error.message));
    return api.onMonitorEvent((event) => {
      if (event.type === "monitor-alert") notify(`${event.label || "监控"}：${event.message}`, "success");
      else setMonitors((previous) => monitorReducer(previous, event));
    });
  }, [api, notify]);
  return <div className="monitor-view">
    {!creating && <div className="operation-actions"><button className="button secondary" onClick={() => setCreating(true)}><Plus size={14} />新建监控</button></div>}
    {creating && <MonitorForm api={api} terminals={terminals} notify={notify} onDone={() => setCreating(false)} />}
    {!monitors.length && !creating && <p className="monitor-note">直接读取主机上的进程、线程、日志或共享内存，不依赖终端屏幕，也不消耗模型调用。</p>}
    {monitors.map((monitor) => <MonitorCard key={monitor.id} monitor={monitor} api={api} notify={notify} />)}
  </div>;
}
