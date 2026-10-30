import { useEffect, useMemo, useRef, useState } from "react";
import { MagnifyingGlass } from "@phosphor-icons/react";
import { fuzzyScore } from "./terminal-utils.js";

export function CommandPalette({ commands, onClose }) {
  const dialog = useRef(null);
  const list = useRef(null);
  const [query, setQuery] = useState("");
  const [index, setIndex] = useState(0);
  useEffect(() => { dialog.current.showModal(); }, []);
  const results = useMemo(() => commands
    .filter((command) => !command.hidden)
    .map((command) => ({ command, score: fuzzyScore(query, `${command.group} ${command.label} ${command.keywords || ""}`) }))
    .filter((item) => item.score > 0)
    .sort((a, b) => b.score - a.score)
    .slice(0, 60)
    .map((item) => item.command), [commands, query]);
  useEffect(() => setIndex(0), [query]);
  useEffect(() => { list.current?.querySelector(".is-selected")?.scrollIntoView({ block: "nearest" }); }, [index]);
  const run = (command) => {
    if (!command || command.disabled) return;
    onClose();
    // Let the dialog close and return focus before the command moves it.
    requestAnimationFrame(() => command.run());
  };
  return <dialog
    ref={dialog}
    className="modal palette-modal"
    onCancel={(event) => { event.preventDefault(); onClose(); }}
    onClick={(event) => { if (event.target === dialog.current) onClose(); }}
  >
    <div className="palette-input">
      <MagnifyingGlass size={16} />
      <input
        autoFocus
        value={query}
        placeholder="输入命令、连接或标签名称…"
        spellCheck={false}
        onChange={(event) => setQuery(event.target.value)}
        onKeyDown={(event) => {
          if (event.key === "ArrowDown") { event.preventDefault(); setIndex((value) => Math.min(results.length - 1, value + 1)); }
          else if (event.key === "ArrowUp") { event.preventDefault(); setIndex((value) => Math.max(0, value - 1)); }
          else if (event.key === "Enter") { event.preventDefault(); run(results[index]); }
        }}
      />
    </div>
    <div className="palette-results" ref={list} role="listbox">
      {results.length ? results.map((command, position) => {
        const Icon = command.icon;
        return <button
          key={command.id}
          type="button"
          role="option"
          aria-selected={position === index}
          className={`palette-item ${position === index ? "is-selected" : ""}`}
          disabled={command.disabled}
          onMouseMove={() => setIndex(position)}
          onClick={() => run(command)}
        >
          {Icon ? <Icon size={16} /> : <span className="palette-icon-placeholder" />}
          <span className="palette-group">{command.group}</span>
          <span className="palette-label">{command.label}</span>
          {command.shortcut && <kbd>{command.shortcut}</kbd>}
        </button>;
      }) : <div className="palette-empty">没有匹配的命令</div>}
    </div>
  </dialog>;
}
