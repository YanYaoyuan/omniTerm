import { useEffect, useRef, useState } from "react";
import { ArrowRight, CircleNotch, Info, PlugsConnected, TerminalWindow, X } from "@phosphor-icons/react";

export const connectionLabels = { ssh: "SSH", sftp: "SFTP", serial: "串口", tunnel: "端口转发" };

export function ConnectionDialog({ initial, onClose, onSave, onConnect }) {
  const dialog = useRef(null);
  const [kind, setKind] = useState(initial.kind || "ssh");
  const [form, setForm] = useState({ name: "", host: "", username: "", port: 22, keyPath: "", device: "", baud: 115200, forwardType: "L", bindPort: "", targetHost: "localhost", targetPort: "", ...initial.profile });
  const [persist, setPersist] = useState(true);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState("");
  useEffect(() => { dialog.current.showModal(); }, []);
  const field = (key) => ({ value: form[key], onChange: (event) => setForm((previous) => ({ ...previous, [key]: event.target.value })) });
  const title = initial.profile ? "编辑连接" : "新建连接";
  const description = {
    ssh: "通过 SSH 安全连接到远程主机。",
    sftp: "打开交互式 SFTP 会话，上传和下载文件。",
    serial: "连接本地串口设备并打开交互控制台。",
    tunnel: "建立 SSH 隧道，安全转发本地或远程端口。",
  }[kind];
  const payload = () => ({
    ...form, kind, name: form.name.trim() || `${connectionLabels[kind]} · ${kind === "serial" ? form.device : form.host}`,
    port: Number(form.port), baud: Number(form.baud),
    ...(kind === "tunnel" ? { bindPort: Number(form.bindPort), ...(form.forwardType !== "D" ? { targetPort: Number(form.targetPort) } : {}) } : {}),
  });
  async function submit(event) {
    event.preventDefault();
    const onlySave = event.nativeEvent.submitter?.value === "save";
    setBusy(true); setError("");
    try {
      const profile = payload();
      if (persist || onlySave) await onSave(profile);
      if (!onlySave) await onConnect(kind, profile);
      onClose();
    } catch (failure) {
      setError(failure.message || String(failure));
    } finally { setBusy(false); }
  }
  return <dialog className="modal connection-modal" ref={dialog} onCancel={(event) => { event.preventDefault(); if (!busy) onClose(); }} onClick={(event) => { if (event.target === dialog.current && !busy) onClose(); }}>
    <form onSubmit={submit}>
      <div className="modal-heading"><div className="modal-heading-icon"><PlugsConnected size={22} /></div><div><h2>{title}</h2><p>配置并管理你的终端连接</p></div><button className="icon-button modal-close" type="button" disabled={busy} onClick={onClose} aria-label="关闭连接窗口"><X size={18} /></button></div>
      <div className="connection-kind" role="tablist" aria-label="连接类型">{Object.entries(connectionLabels).map(([value, label]) => <button key={value} type="button" role="tab" aria-selected={kind === value} className={kind === value ? "active" : ""} onClick={() => setKind(value)} disabled={busy}>{label}</button>)}</div>
      <p className="form-description">{description}</p>
      <div className="form-fields">
        <label className="field full">连接名称<input {...field("name")} placeholder="为这个连接起个名字" autoFocus maxLength={80} /></label>
        {kind !== "serial" ? <>
          <label className="field host">主机地址<input {...field("host")} placeholder="主机名或 IP 地址" required spellCheck={false} /></label>
          <label className="field port">SSH 端口<input {...field("port")} type="number" min="1" max="65535" required /></label>
          <label className="field full">用户名<input {...field("username")} placeholder="留空时使用系统默认用户" spellCheck={false} /></label>
          <label className="field full">私钥路径 <span className="optional">可选</span><input {...field("keyPath")} placeholder="例如 ~/.ssh/id_ed25519" spellCheck={false} /><span className="field-note">也可以使用系统 SSH 配置和 SSH Agent。</span></label>
        </> : <>
          <label className="field host">串口设备<input {...field("device")} placeholder="/dev/ttyUSB0 或 COM3" required spellCheck={false} /></label>
          <label className="field port">波特率<input {...field("baud")} type="number" min="1" max="4000000" required /></label>
          <div className="form-notice full"><Info size={16} /><span>使用本地 Python 与 pyserial。连接后按 Ctrl+] 退出串口控制台。</span></div>
        </>}
        {kind === "tunnel" && <>
          <div className="form-divider full" />
          <label className="field host">转发类型<select {...field("forwardType")}><option value="L">本地转发（-L）</option><option value="R">远程转发（-R）</option><option value="D">动态 SOCKS（-D）</option></select></label>
          <label className="field port">监听端口<input {...field("bindPort")} type="number" min="1" max="65535" placeholder="8080" required /></label>
          {form.forwardType !== "D" && <><label className="field host">目标主机<input {...field("targetHost")} required spellCheck={false} /></label><label className="field port">目标端口<input {...field("targetPort")} type="number" min="1" max="65535" placeholder="3000" required /></label></>}
        </>}
      </div>
      <label className="checkbox-field"><input type="checkbox" checked={persist} onChange={(event) => setPersist(event.target.checked)} />保存到我的连接</label>
      {error && <div className="inline-error" role="alert">{error}</div>}
      <div className="modal-actions"><button className="button secondary" type="submit" value="save" disabled={busy}>仅保存</button><div className="action-group"><button className="button ghost" type="button" onClick={onClose} disabled={busy}>取消</button><button className="button primary" type="submit" value="connect" disabled={busy}>{busy ? <CircleNotch size={16} className="spin" /> : <TerminalWindow size={16} />}连接<ArrowRight size={14} /></button></div></div>
    </form>
  </dialog>;
}
