import { useCallback, useEffect, useRef, useState, useSyncExternalStore } from "react";
import {
  ArrowClockwise, ArrowDown, ArrowLeft, ArrowUp, CircleNotch, Copy, DownloadSimple, Eye, EyeSlash, File, Folder,
  FolderPlus, HardDrives, House, Link, PencilSimple, Trash, UploadSimple, WarningCircle, X,
} from "@phosphor-icons/react";
import { formatBytes, formatTime, formatUptime, joinRemote, nextSelection, parentRemote, percent, sortEntries, transferFinished, validEntryName } from "./remote-utils.js";

const UNAVAILABLE = {
  "no-sftp": "这台主机没有启用 SFTP，文件面板不可用。",
  auth: "无法复用 SSH 登录。Windows 上文件面板需要密钥或 SSH Agent 认证。",
  lost: "SFTP 连接已断开。",
  failed: "无法打开 SFTP 连接。",
};
const SYMLINK_MASK = 0o777;
const permissions = (mode) => (mode & SYMLINK_MASK).toString(8).padStart(3, "0");

function ConfirmDialog({ title, detail, action, danger, onCancel, onConfirm }) {
  const dialog = useRef(null);
  useEffect(() => { dialog.current.showModal(); }, []);
  return <dialog ref={dialog} className="modal delete-modal" onCancel={(event) => { event.preventDefault(); onCancel(); }}>
    <div className="modal-heading"><div className="modal-heading-icon"><WarningCircle size={22} /></div><div><h2>{title}</h2></div></div>
    <p className="delete-explanation sftp-confirm-detail">{detail}</p>
    <div className="modal-actions"><span /><div className="action-group"><button className="button ghost" onClick={onCancel}>取消</button><button className={`button ${danger ? "danger" : "primary"}`} onClick={onConfirm}>{action}</button></div></div>
  </dialog>;
}

function NameInput({ initial, placeholder, onCommit, onCancel }) {
  const [value, setValue] = useState(initial);
  const input = useRef(null);
  const done = useRef(false);
  useEffect(() => { input.current.focus(); input.current.select(); }, []);
  const finish = (commit) => {
    if (done.current) return;
    done.current = true;
    if (commit && value.trim() && value !== initial) onCommit(value.trim()); else onCancel();
  };
  return <input ref={input} className="sftp-name-input" value={value} placeholder={placeholder} maxLength={255} spellCheck={false} onChange={(event) => setValue(event.target.value)} onBlur={() => finish(true)} onClick={(event) => event.stopPropagation()} onDoubleClick={(event) => event.stopPropagation()} onKeyDown={(event) => { event.stopPropagation(); if (event.key === "Enter") finish(true); if (event.key === "Escape") finish(false); }} />;
}

function Transfers({ store, id, api, notify }) {
  const transfers = useSyncExternalStore(store.subscribe, () => store.transfers(id));
  if (!transfers.length) return null;
  return <div className="sftp-transfers" aria-label="传输队列">
    <div className="sftp-transfers-heading"><span>传输</span>{transfers.some(transferFinished) && <button onClick={() => store.clearFinished(id)}>清除已完成</button>}</div>
    {transfers.map((transfer) => {
      const value = percent(transfer.bytes, transfer.total);
      const Icon = transfer.direction === "upload" ? ArrowUp : ArrowDown;
      const status = { queued: "排队中", done: `完成${transfer.skipped ? `，跳过 ${transfer.skipped} 项` : ""}`, canceled: "已取消", failed: "失败" }[transfer.status] ?? (value === null ? "准备中" : `${Math.round(value)}%`);
      return <div className={`sftp-transfer ${transfer.status}`} key={transfer.transferId} title={transfer.error || transfer.name}>
        <Icon size={13} /><span className="sftp-transfer-name">{transfer.name}</span><span className="sftp-transfer-status">{status}</span>
        {transferFinished(transfer) ? null : <button className="icon-button" title="取消传输" aria-label={`取消传输 ${transfer.name}`} onClick={() => Promise.resolve(api.remoteCancel({ id, transferId: transfer.transferId })).catch((error) => notify(error.message))}><X size={12} /></button>}
        {transfer.status === "running" && <div className="sftp-transfer-bar"><span style={{ width: `${value ?? 0}%` }} /></div>}
        {transfer.status === "failed" && <div className="sftp-transfer-error">{transfer.error}</div>}
      </div>;
    })}
  </div>;
}

export function SftpPanel({ session, remote, store, api, notify, memory }) {
  const { id } = session;
  const saved = memory.get(id);
  const [path, setPath] = useState(saved?.path ?? null);
  const [draft, setDraft] = useState(saved?.path ?? "");
  const [entries, setEntries] = useState([]);
  const [loading, setLoading] = useState(false);
  const [error, setError] = useState("");
  const [selection, setSelection] = useState(new Set());
  const [showHidden, setShowHidden] = useState(saved?.showHidden ?? false);
  const [editing, setEditing] = useState(null);
  const [dropTarget, setDropTarget] = useState(null);
  const [confirm, setConfirm] = useState(null);
  const [menu, setMenu] = useState(null);
  const [preparing, setPreparing] = useState(new Set());
  const [prepared, setPrepared] = useState(() => saved?.prepared ?? new Map());
  const anchor = useRef(null);
  const request = useRef(0);
  const ready = remote.state === "ready";
  const transfers = useSyncExternalStore(store.subscribe, () => store.transfers(id));
  const finishedUploads = transfers.filter((transfer) => transfer.direction === "upload" && transfer.status === "done").length;

  const load = useCallback(async (target) => {
    const ticket = ++request.current;
    setLoading(true);
    try {
      const result = await api.remoteList({ id, path: target });
      if (ticket !== request.current) return;
      setPath(result.path); setDraft(result.path); setEntries(result.entries); setError(""); setSelection(new Set());
    } catch (failure) {
      if (ticket !== request.current) return;
      setError(failure.message || String(failure));
      setPath((current) => { setDraft(current ?? target); return current; });
    } finally { if (ticket === request.current) setLoading(false); }
  }, [api, id]);

  useEffect(() => { if (ready) load(path ?? remote.home); }, [ready]);
  useEffect(() => { if (ready && path && finishedUploads) load(path); }, [finishedUploads]);
  useEffect(() => { memory.set(id, { path, showHidden, prepared }); }, [memory, id, path, showHidden, prepared]);
  useEffect(() => {
    if (!menu) return undefined;
    const close = () => setMenu(null);
    window.addEventListener("click", close);
    return () => window.removeEventListener("click", close);
  }, [menu]);

  const guarded = (task) => Promise.resolve().then(task).catch((failure) => notify(failure.message || String(failure)));
  const visible = sortEntries(entries, showHidden);
  const selected = visible.filter((entry) => selection.has(entry.name));
  const fullPath = (entry) => joinRemote(path, entry.name);
  const dragKey = (entry) => `${fullPath(entry)}:${entry.size}:${entry.mtime}`;

  const upload = (paths, directory, overwrite = false) => guarded(async () => {
    const result = await api.remoteUpload({ id, paths, directory, overwrite });
    if (result.conflicts) setConfirm({ title: "覆盖远程文件？", detail: `${directory} 中已存在：${result.conflicts.join("、")}`, action: "覆盖", danger: true, run: () => upload(paths, directory, true) });
  });
  const download = (items, destination, overwrite = false) => guarded(async () => {
    const directory = destination ?? await api.remoteChooseDownload();
    if (!directory) return;
    const result = await api.remoteDownload({ id, paths: items.map(fullPath), destination: directory, overwrite });
    if (result.conflicts) setConfirm({ title: "覆盖本地文件？", detail: `${directory} 中已存在：${result.conflicts.join("、")}`, action: "覆盖", danger: true, run: () => download(items, directory, true) });
  });
  const remove = (items) => setConfirm({
    title: `删除 ${items.length} 个远程项目？`,
    detail: `${items.map((item) => item.name).join("、")}\n此操作无法撤销，文件夹会连同其内容一起删除。`,
    action: "删除", danger: true,
    run: () => guarded(async () => { await api.remoteRemove({ id, paths: items.map(fullPath) }); await load(path); }),
  });
  const commitName = (name) => {
    const edit = editing;
    setEditing(null);
    if (!validEntryName(name)) { notify("名称不能为空，也不能包含 /。"); return; }
    guarded(async () => {
      if (edit.kind === "folder") await api.remoteMkdir({ id, path: joinRemote(path, name) });
      else await api.remoteRename({ id, from: fullPath(edit.entry), to: joinRemote(path, name) });
      await load(path);
    });
  };
  const open = (entry) => { if (entry.directory) load(fullPath(entry)); else download([entry]); };
  const clickRow = (event, entry) => {
    const names = visible.map((item) => item.name);
    setSelection(nextSelection(selection, names, entry.name, { toggle: event.metaKey || event.ctrlKey, range: event.shiftKey }, anchor.current));
    if (!event.shiftKey) anchor.current = entry.name;
  };
  // The OS can only drag a finished local file, so the first drag fetches the
  // item and marks its row; the next drag from that row is the native one.
  const dragRow = (event, entry) => {
    event.preventDefault();
    const key = dragKey(entry);
    const token = prepared.get(key);
    if (token) { api.remoteStartDrag({ id, token }); return; }
    if (preparing.has(key)) return;
    setPreparing((current) => new Set(current).add(key));
    guarded(async () => {
      try {
        const result = await api.remotePrepareDrag({ id, path: fullPath(entry) });
        setPrepared((current) => new Map(current).set(key, result.token));
        notify(`${entry.name} 已准备好，再次拖动即可拖到本地文件夹。`, "success");
      } finally { setPreparing((current) => { const next = new Set(current); next.delete(key); return next; }); }
    });
  };
  const acceptsFiles = (event) => event.dataTransfer?.types?.includes("Files");
  const dropFiles = (event, directory) => {
    event.preventDefault();
    event.stopPropagation();
    setDropTarget(null);
    const paths = [...(event.dataTransfer?.files || [])].map((file) => api.pathForFile(file)).filter(Boolean);
    if (paths.length && ready && path) upload(paths, directory);
  };
  const contextMenu = (event, entry) => {
    event.preventDefault();
    if (!selection.has(entry.name)) { setSelection(new Set([entry.name])); anchor.current = entry.name; }
    setMenu({ x: Math.min(event.clientX, window.innerWidth - 170), y: Math.min(event.clientY, window.innerHeight - 190), entry });
  };

  if (!ready) {
    return <section className="sftp-panel sftp-placeholder" aria-label="SFTP 文件面板">
      {remote.state === "unavailable"
        ? <><WarningCircle size={22} /><p>{UNAVAILABLE[remote.reason] ?? UNAVAILABLE.failed}</p>{remote.detail && remote.reason !== "no-sftp" && <pre>{remote.detail}</pre>}<button className="button secondary" onClick={() => guarded(() => api.remoteRetry(id))}><ArrowClockwise size={14} />重试</button></>
        : <><CircleNotch className="spin" size={20} /><p>{remote.state === "waiting" ? "等待 SSH 登录完成…" : "正在打开 SFTP…"}</p></>}
    </section>;
  }

  return <section className={`sftp-panel ${dropTarget === "panel" ? "is-drop-target" : ""}`} aria-label="SFTP 文件面板"
    onDragOver={(event) => { if (acceptsFiles(event)) { event.preventDefault(); event.dataTransfer.dropEffect = "copy"; if (dropTarget === null) setDropTarget("panel"); } }}
    onDragLeave={(event) => { if (!event.currentTarget.contains(event.relatedTarget)) setDropTarget(null); }}
    onDrop={(event) => dropFiles(event, path)}>
    <div className="sftp-toolbar">
      <button className="icon-button" title="上一级" aria-label="上一级" disabled={path === "/"} onClick={() => load(parentRemote(path))}><ArrowLeft size={15} /></button>
      <button className="icon-button" title="主目录" aria-label="主目录" onClick={() => load(remote.home)}><House size={15} /></button>
      <button className="icon-button" title="刷新" aria-label="刷新" onClick={() => load(path)}><ArrowClockwise size={15} className={loading ? "spin" : ""} /></button>
      <span className="toolbar-separator" />
      <button className="icon-button" title="上传文件" aria-label="上传文件" onClick={() => guarded(async () => { const paths = await api.remoteChooseUpload(); if (paths.length) await upload(paths, path); })}><UploadSimple size={15} /></button>
      <button className="icon-button" title="新建文件夹" aria-label="新建文件夹" onClick={() => setEditing({ kind: "folder" })}><FolderPlus size={15} /></button>
      <button className={`icon-button ${showHidden ? "is-active" : ""}`} title={showHidden ? "隐藏点文件" : "显示点文件"} aria-label="切换显示点文件" aria-pressed={showHidden} onClick={() => setShowHidden((value) => !value)}>{showHidden ? <Eye size={15} /> : <EyeSlash size={15} />}</button>
    </div>
    <form className="sftp-path" onSubmit={(event) => { event.preventDefault(); if (draft.trim()) load(draft.trim()); }}><input aria-label="远程路径" value={draft} spellCheck={false} onChange={(event) => setDraft(event.target.value)} /></form>
    {error && <div className="sftp-error" role="alert"><WarningCircle size={14} /><span>{error}</span></div>}
    <div className="sftp-list" role="listbox" aria-multiselectable="true" aria-label="远程文件">
      {editing?.kind === "folder" && <div className="sftp-row"><Folder size={16} weight="fill" className="sftp-folder" /><NameInput initial="" placeholder="文件夹名称" onCommit={commitName} onCancel={() => setEditing(null)} /></div>}
      {visible.map((entry) => {
        const Icon = entry.directory ? Folder : File;
        const key = dragKey(entry);
        const isDrop = dropTarget === entry.name;
        return <div key={entry.name} role="option" aria-selected={selection.has(entry.name)} tabIndex={0} draggable title={`${entry.name}\n${entry.directory ? "文件夹" : formatBytes(entry.size)} · ${formatTime(entry.mtime)} · ${permissions(entry.mode)}`}
          className={`sftp-row ${selection.has(entry.name) ? "selected" : ""} ${isDrop ? "is-drop-target" : ""}`}
          onClick={(event) => clickRow(event, entry)} onDoubleClick={() => open(entry)} onContextMenu={(event) => contextMenu(event, entry)}
          onKeyDown={(event) => { if (event.key === "Enter") open(entry); if (event.key === "Delete") remove(selected.length ? selected : [entry]); }}
          onDragStart={(event) => dragRow(event, entry)}
          onDragOver={entry.directory ? (event) => { if (acceptsFiles(event)) { event.preventDefault(); event.stopPropagation(); event.dataTransfer.dropEffect = "copy"; setDropTarget(entry.name); } } : undefined}
          onDrop={entry.directory ? (event) => dropFiles(event, fullPath(entry)) : undefined}>
          <Icon size={16} weight={entry.directory ? "fill" : "regular"} className={entry.directory ? "sftp-folder" : ""} />
          {editing?.entry?.name === entry.name
            ? <NameInput initial={entry.name} onCommit={commitName} onCancel={() => setEditing(null)} />
            : <span className="sftp-name">{entry.name}</span>}
          {entry.symlink && <Link size={12} className="sftp-link" />}
          {preparing.has(key) ? <CircleNotch className="spin" size={13} /> : prepared.has(key) ? <span className="sftp-ready" title="已准备好，可拖到本地文件夹">可拖出</span> : !entry.directory && <span className="sftp-size">{formatBytes(entry.size)}</span>}
        </div>;
      })}
      {!visible.length && !editing && !loading && <div className="sftp-empty"><HardDrives size={20} /><span>{entries.length ? "只有隐藏文件" : "空文件夹"}</span></div>}
    </div>
    {selected.length > 0 && <div className="sftp-selection"><span>已选 {selected.length} 项</span><button className="icon-button" title="下载" aria-label="下载选中项" onClick={() => download(selected)}><DownloadSimple size={15} /></button><button className="icon-button" title="删除" aria-label="删除选中项" onClick={() => remove(selected)}><Trash size={15} /></button></div>}
    <Transfers store={store} id={id} api={api} notify={notify} />
    <div className="sftp-hint"><UploadSimple size={12} /><span>拖入文件上传 · 拖动列表项下载到本地</span></div>
    {menu && <div className="sftp-menu" style={{ left: menu.x, top: menu.y }} role="menu">
      {menu.entry.directory && <button role="menuitem" onClick={() => open(menu.entry)}><Folder size={14} />打开</button>}
      <button role="menuitem" onClick={() => download(selected.length ? selected : [menu.entry])}><DownloadSimple size={14} />下载…</button>
      <button role="menuitem" onClick={() => setEditing({ kind: "rename", entry: menu.entry })}><PencilSimple size={14} />重命名</button>
      <button role="menuitem" onClick={() => guarded(() => navigator.clipboard.writeText(fullPath(menu.entry)))}><Copy size={14} />复制路径</button>
      <button role="menuitem" onClick={() => remove(selected.length ? selected : [menu.entry])}><Trash size={14} />删除</button>
    </div>}
    {confirm && <ConfirmDialog {...confirm} onCancel={() => setConfirm(null)} onConfirm={() => { const { run } = confirm; setConfirm(null); run(); }} />}
  </section>;
}

function Meter({ label, value, detail, title }) {
  return <div className="metric" title={[title, detail].filter(Boolean).join(" · ") || undefined}>
    <span className="metric-label">{label}</span>
    <div className="metric-meter" role="meter" aria-label={label} aria-valuemin={0} aria-valuemax={100} aria-valuenow={value === null ? undefined : Math.round(value)}><span style={{ width: `${value ?? 0}%` }} /></div>
    <span className="metric-value">{value === null ? "—" : `${Math.round(value)}%`}</span>
    {detail && <span className="metric-detail">{detail}</span>}
  </div>;
}

export function MetricsBar({ session, remote, store, api, notify }) {
  const { id } = session;
  const metrics = useSyncExternalStore(store.subscribe, () => store.metrics(id));
  useEffect(() => {
    Promise.resolve(api.remoteWatchMetrics({ id, enabled: true })).catch((error) => notify(error.message));
    return () => { Promise.resolve(api.remoteWatchMetrics({ id, enabled: false })).catch(() => { /* The session may have closed first. */ }); };
  }, [api, id, notify]);
  const body = () => {
    if (remote.state === "waiting") return <span className="metrics-note">等待 SSH 登录完成…</span>;
    if (metrics && !metrics.available) return <><span className="metrics-note">主机监控不可用：远程主机需要提供 POSIX shell。</span><button className="metrics-retry" onClick={() => Promise.resolve(api.remoteRetry(id)).catch((error) => notify(error.message))}>重试</button></>;
    if (!metrics?.sample) return <span className="metrics-note"><CircleNotch className="spin" size={12} />正在采集主机信息…</span>;
    const { sample } = metrics;
    return <>
      <Meter label="CPU" value={sample.cpu ?? null} detail={sample.cores ? `${sample.cores} 核` : null} title={sample.load ? `平均负载 ${sample.load.join(" / ")}` : undefined} />
      {sample.mem && <Meter label="内存" value={percent(sample.mem.used, sample.mem.total)} detail={`${formatBytes(sample.mem.used)} / ${formatBytes(sample.mem.total)}`} title={sample.swap ? `交换分区 ${formatBytes(sample.swap.used)} / ${formatBytes(sample.swap.total)}` : undefined} />}
      {sample.gpus.map((gpu, index) => <Meter key={index} label={sample.gpus.length > 1 ? `GPU ${index}` : "GPU"} value={gpu.util} detail={[gpu.memTotal ? `${formatBytes(gpu.memUsed)} / ${formatBytes(gpu.memTotal)}` : null, gpu.temp !== null ? `${gpu.temp}°C` : null].filter(Boolean).join(" · ") || null} title={gpu.name} />)}
      {sample.disk && <Meter label="磁盘 /" value={percent(sample.disk.used, sample.disk.total)} detail={`${formatBytes(sample.disk.used)} / ${formatBytes(sample.disk.total)}`} />}
      {sample.net && <div className="metric" title="网络吞吐量"><ArrowDown size={11} /><span className="metric-value">{formatBytes(sample.net.rx)}/s</span><ArrowUp size={11} /><span className="metric-value">{formatBytes(sample.net.tx)}/s</span></div>}
      {sample.load && <div className="metric"><span className="metric-label">负载</span><span className="metric-value">{sample.load[0].toFixed(2)}</span></div>}
      {sample.uptime !== undefined && <div className="metric"><span className="metric-label">运行</span><span className="metric-value">{formatUptime(sample.uptime)}</span></div>}
    </>;
  };
  return <div className="metrics-bar" role="status" aria-label="远程主机监控"><HardDrives size={13} /><span className="metrics-host">{session.profile?.name ?? session.title}</span>{body()}</div>;
}
