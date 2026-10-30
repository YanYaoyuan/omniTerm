import { useEffect, useRef, useState } from "react";
import { CheckCircle, CircleNotch, Plugs, Plus, Trash, WarningCircle, X } from "@phosphor-icons/react";
import { WIRE_LABELS } from "./agent-logic.js";

const blank = (presets, presetId = "qwen") => {
  const preset = presets[presetId];
  return { presetId, name: preset.label, wireApi: preset.wireApi, baseUrl: preset.baseUrl, model: "", chatOnly: false, contextBudgetBytes: 64 * 1024 };
};

export function ProviderDialog({ api, state, onState, onClose, notify }) {
  const dialog = useRef(null);
  const [selected, setSelected] = useState(state.profiles[0]?.id ?? null);
  const current = state.profiles.find((profile) => profile.id === selected);
  const [form, setForm] = useState(current ?? blank(state.presets));
  const [apiKey, setApiKey] = useState("");
  const [busy, setBusy] = useState("");
  const [error, setError] = useState("");
  useEffect(() => { dialog.current.showModal(); }, []);
  const preset = state.presets[form.presetId];
  const engineLogin = preset.auth === "engine-login";
  const update = (key) => (event) => setForm((previous) => ({ ...previous, [key]: event.target.type === "checkbox" ? event.target.checked : event.target.value }));
  const choose = (profile) => { setSelected(profile?.id ?? null); setForm(profile ?? blank(state.presets)); setApiKey(""); setError(""); };
  const changePreset = (event) => { const next = blank(state.presets, event.target.value); setForm({ ...next, id: form.id, model: form.model }); };
  async function run(label, task) {
    setBusy(label); setError("");
    try { return await task(); }
    catch (failure) { setError(failure.message || String(failure)); return null; }
    finally { setBusy(""); }
  }
  const save = () => run("save", async () => {
    const result = await api.saveProvider({ profile: form, ...(apiKey ? { apiKey } : {}) });
    onState(result.state); setSelected(result.saved.id); setForm(result.saved); setApiKey("");
    return result.saved;
  });
  const test = () => run("test", async () => {
    const saved = await api.saveProvider({ profile: form, ...(apiKey ? { apiKey } : {}) });
    setApiKey("");
    const next = await api.testProvider(saved.saved.id);
    onState(next);
    setSelected(saved.saved.id);
    setForm(next.profiles.find((profile) => profile.id === saved.saved.id) ?? saved.saved);
    notify("模型服务测试通过。", "success");
  });
  const remove = () => run("delete", async () => { const next = await api.deleteProvider(selected); onState(next); choose(next.profiles[0] ?? null); });
  const verified = current?.verified;
  return <dialog ref={dialog} className="modal provider-modal" onCancel={(event) => { event.preventDefault(); onClose(selected); }}>
    <div className="modal-heading"><div className="modal-heading-icon"><Plugs size={22} /></div><div><h2>omniTerm Agent 模型设置</h2><p>使用你自己的模型服务，不需要 OpenAI 账号。关闭时选中的配置用于之后的对话。</p></div><button type="button" className="icon-button modal-close" aria-label="关闭模型设置" onClick={() => onClose(selected)}><X size={18} /></button></div>
    <div className="provider-layout">
      <div className="provider-list">
        {state.profiles.map((profile) => <button key={profile.id} className={profile.id === selected ? "active" : ""} onClick={() => choose(profile)}><strong>{profile.name}</strong><small>{profile.model}</small></button>)}
        <button className={!selected ? "active" : ""} onClick={() => choose(null)}><Plus size={14} /><span>添加配置</span></button>
      </div>
      <div className="provider-form">
        <label>服务商<select value={form.presetId} onChange={changePreset}>{Object.entries(state.presets).map(([id, item]) => <option key={id} value={id}>{item.label}</option>)}</select></label>
        <label>配置名称<input value={form.name} maxLength={120} onChange={update("name")} /></label>
        <label>协议<select value={form.wireApi} onChange={update("wireApi")}>{preset.protocols.map((protocol) => <option key={protocol} value={protocol}>{WIRE_LABELS[protocol]}</option>)}</select></label>
        {!engineLogin && <label>Base URL<input value={form.baseUrl} placeholder="https://…" onChange={update("baseUrl")} /></label>}
        <label>模型 ID<input value={form.model} placeholder="按服务商文档填写" onChange={update("model")} /></label>
        {!engineLogin && <label>API Key<input type="password" autoComplete="off" value={apiKey} placeholder={current?.hasCredential ? "已保存，留空则不修改" : "粘贴 API Key"} onChange={(event) => setApiKey(event.target.value)} /></label>}
        {!engineLogin && <p className="provider-note">{state.credentialStorage === "os" ? "密钥由系统凭据服务加密保存，不会写入配置或交给界面。" : "系统凭据服务不可用：密钥只保存在本次运行的内存中，退出后需要重新填写。"}</p>}
        {engineLogin && <p className="provider-note">使用引擎已有的登录状态。此配置读取你现有的引擎主目录。</p>}
        <label className="provider-check"><input type="checkbox" checked={form.chatOnly} onChange={update("chatOnly")} />仅聊天（模型不支持工具调用时使用；不能读取或操作终端）</label>
        {verified && <div className="provider-verified">{verified.connection ? <span><CheckCircle size={14} />连接已验证</span> : <span className="muted"><WarningCircle size={14} />连接未验证</span>}{verified.tools ? <span><CheckCircle size={14} />工具调用已验证</span> : <span className="muted"><WarningCircle size={14} />工具调用未验证</span>}</div>}
        {error && <div className="inline-error">{error}</div>}
      </div>
    </div>
    <div className="modal-actions">{selected ? <button className="button ghost" disabled={Boolean(busy)} onClick={remove}><Trash size={15} />删除</button> : <span />}<div className="action-group"><button className="button secondary" disabled={Boolean(busy)} onClick={test}>{busy === "test" && <CircleNotch className="spin" size={15} />}测试连接与工具</button><button className="button primary" disabled={Boolean(busy)} onClick={save}>保存</button></div></div>
  </dialog>;
}
