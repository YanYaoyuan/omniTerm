import { useEffect, useMemo, useState } from "react";
import { sparkline } from "../agent/monitor-logic.js";
import { formatMemory, monitorRequest, processRows, pushTrends, summarize } from "./process-logic.js";

const api = window.omniterm;
const REFRESH_MS = 3000;

function Trend({ series, field, label }) {
  const points = sparkline(series, field);
  return <div className="process-trend">{label}{points ? <svg viewBox="0 0 100 26" preserveAspectRatio="none" role="img" aria-label={label}><polyline fill="none" stroke="currentColor" strokeWidth="1" vectorEffect="non-scaling-stroke" points={points} /></svg> : <div>正在收集…</div>}</div>;
}

export function ProcessWindow() {
  const [host, setHost] = useState(() => new URLSearchParams(location.search).get("host") || "local");
  const [hosts, setHosts] = useState([]);
  const [processes, setProcesses] = useState([]);
  const [trends, setTrends] = useState(new Map());
  const [query, setQuery] = useState("");
  const [sort, setSort] = useState(null);
  const [selected, setSelected] = useState(null);
  const [error, setError] = useState("");
  const [message, setMessage] = useState("");

  useEffect(() => { api.listWorkspace().then(({ terminals }) => setHosts(terminals.filter(({ kind }) => kind === "ssh"))).catch(() => {}); }, []);
  useEffect(() => {
    let active = true;
    let timer;
    setProcesses([]); setTrends(new Map()); setSelected(null); setError("");
    const refresh = async () => {
      try {
        const list = await api.processSnapshot(host);
        if (!active) return;
        setProcesses(list); setTrends((previous) => pushTrends(previous, list)); setError("");
      } catch (failure) { if (active) setError(failure.message); }
      if (active) timer = setTimeout(refresh, REFRESH_MS);
    };
    refresh();
    return () => { active = false; clearTimeout(timer); };
  }, [host]);

  const rows = useMemo(() => processRows(processes, { query, sort }), [processes, query, sort]);
  const summary = summarize(processes);
  const chosen = processes.find(({ pid }) => pid === selected);
  async function attach() {
    try {
      await api.startProcessMonitor(monitorRequest(chosen, host));
      setMessage(`已开始监控 ${chosen.command}，可在 omniTerm Agent 面板的“监控”中查看。`);
    } catch (failure) { setError(failure.message); }
  }
  const header = (field, label, numeric = true) => <th className={numeric ? "number" : ""} onClick={() => setSort(sort === field ? null : field)} aria-sort={sort === field ? "descending" : "none"}>{label}{sort === field ? " ↓" : ""}</th>;
  return <div className="process-app">
    <div className="process-bar">
      <select aria-label="主机" value={host} onChange={(event) => setHost(event.target.value)}><option value="local">本机</option>{hosts.map((terminal) => <option key={terminal.ref.sessionId} value={terminal.ref.sessionId}>{terminal.label}</option>)}</select>
      <input aria-label="搜索进程" placeholder="按名称或 PID 搜索" value={query} onChange={(event) => setQuery(event.target.value)} />
      <span className="process-summary">{summary.count} 个进程 · CPU {summary.cpu}% · 内存 {formatMemory(summary.memoryKb)}</span>
    </div>
    <div className="process-table">
      {error && <div className="process-error">{error}</div>}
      <table>
        <thead><tr><th onClick={() => setSort(null)}>进程{sort ? "（点击恢复树状）" : ""}</th>{header("pid", "PID")}{header("cpu", "CPU %")}{header("memoryKb", "内存")}</tr></thead>
        <tbody>{rows.map((process) => <tr key={process.pid} className={process.pid === selected ? "selected" : ""} onClick={() => setSelected(process.pid)} aria-selected={process.pid === selected}><td style={{ paddingLeft: 10 + process.depth * 14 }}>{process.command}</td><td className="number">{process.pid}</td><td className="number">{process.cpu.toFixed(1)}</td><td className="number">{formatMemory(process.memoryKb)}</td></tr>)}</tbody>
      </table>
    </div>
    {chosen && <div className="process-detail">
      <Trend series={trends.get(chosen.pid)} field="cpu" label={`${chosen.command} CPU 趋势`} />
      <Trend series={trends.get(chosen.pid)} field="memoryKb" label="内存趋势" />
      <div className="process-actions"><button className="button primary" onClick={attach}>持续监控</button>{message && <span className="process-summary">{message}</span>}</div>
    </div>}
  </div>;
}
