import { useEffect, useRef, useState } from "react";
import { Terminal } from "@xterm/xterm";
import { FitAddon } from "@xterm/addon-fit";
import { SearchAddon } from "@xterm/addon-search";
import { CircleNotch, WarningCircle } from "@phosphor-icons/react";

export function TerminalView({ session, visible, settings, api, register, onState, onError, onFocus }) {
  const host = useRef(null);
  const terminal = useRef(null);
  const fitRef = useRef(null);
  const callbacks = useRef({ onState, onError, onFocus });
  const [state, setState] = useState("opening");
  const [failure, setFailure] = useState("");
  callbacks.current = { onState, onError, onFocus };

  useEffect(() => {
    let disposed = false;
    let exited = false;
    let frame;
    const term = new Terminal({
      cursorBlink: settings.cursorBlink,
      cursorStyle: "bar",
      fontSize: settings.fontSize,
      fontFamily: '"SFMono-Regular", Menlo, Monaco, Consolas, "Liberation Mono", monospace',
      lineHeight: 1.22,
      letterSpacing: 0,
      scrollback: settings.scrollback,
      allowProposedApi: false,
      theme: {
        background: "#14181f", foreground: "#e0e6ee", cursor: "#8db4ff",
        cursorAccent: "#14181f", selectionBackground: "#8db4ff36",
        black: "#39414e", red: "#e58a91", green: "#a5c9a1", yellow: "#dec38d",
        blue: "#8db4ff", magenta: "#bdacd8", cyan: "#92c4cc", white: "#e0e6ee",
        brightBlack: "#7d899a", brightRed: "#f4a2a8", brightGreen: "#b9dab5",
        brightYellow: "#efd5a5", brightBlue: "#accaff", brightMagenta: "#d4c3ed",
        brightCyan: "#aedce3", brightWhite: "#f5f7fa",
      },
    });
    const fit = new FitAddon();
    const search = new SearchAddon();
    term.loadAddon(fit);
    term.loadAddon(search);
    term.open(host.current);
    terminal.current = term;
    fitRef.current = fit;

    const report = (error) => {
      if (!disposed) callbacks.current.onError(error?.message || String(error));
    };
    const write = term.onData((data) => {
      if (!exited) Promise.resolve(api.writeSession({ id: session.id, data })).catch(report);
    });
    const resized = term.onResize(({ cols, rows }) => {
      Promise.resolve(api.resizeSession({ id: session.id, cols, rows })).catch(report);
    });
    const resize = () => {
      if (disposed || !host.current?.offsetWidth || !host.current?.offsetHeight) return;
      try {
        fit.fit();
        Promise.resolve(api.resizeSession({ id: session.id, cols: term.cols, rows: term.rows })).catch(report);
      } catch (error) { report(error); }
    };
    const observer = new ResizeObserver(() => {
      cancelAnimationFrame(frame);
      frame = requestAnimationFrame(resize);
    });
    observer.observe(host.current);
    const offData = api.onSessionData(({ id, data }) => {
      if (id === session.id && !disposed) term.write(data);
    });
    const finish = ({ exitCode, signal }) => {
      if (disposed || exited) return;
      exited = true;
      setState("exited");
      callbacks.current.onState(session.id, "exited", exitCode);
      term.writeln(`\r\n\x1b[38;2;139;152;170m进程已退出${exitCode != null ? ` · 退出码 ${exitCode}` : ""}${signal ? ` · ${signal}` : ""}\x1b[0m`);
    };
    const offExit = api.onSessionExit((event) => {
      if (event.id === session.id) finish(event);
    });
    const controls = {
      focus: () => term.focus(),
      find: (value, previous = false) => {
        if (!value) { search.clearDecorations(); term.clearSelection(); return true; }
        return previous ? search.findPrevious(value) : search.findNext(value);
      },
      clearSearch: () => { search.clearDecorations(); term.clearSelection(); },
      copy: async () => {
        const selection = term.getSelection();
        if (!selection) throw new Error("请先在终端中选择要复制的文本。");
        await navigator.clipboard.writeText(selection);
      },
      paste: async () => {
        if (exited) throw new Error("该会话已结束，请新建终端后再粘贴。");
        term.paste(await navigator.clipboard.readText());
        term.focus();
      },
      clear: () => { term.clear(); term.focus(); },
    };
    register(session.id, controls);
    term.attachCustomKeyEventHandler((event) => {
      if (event.type !== "keydown") return true;
      const key = event.key.toLowerCase();
      if ((event.metaKey || event.ctrlKey) && ["t", "w", "f", "tab"].includes(key)) return false;
      const clipboardShortcut = event.metaKey || (event.ctrlKey && event.shiftKey);
      if (clipboardShortcut && key === "c" && term.hasSelection()) {
        event.preventDefault(); controls.copy().catch(report); return false;
      }
      if (clipboardShortcut && key === "v") {
        event.preventDefault(); controls.paste().catch(report); return false;
      }
      return true;
    });
    async function start() {
      try {
        // New sessions stay pending until this view has registered its listeners.
        const buffer = await api.getSessionBuffer(session.id);
        if (disposed) return;
        if (buffer.data) term.write(buffer.data);
        if (buffer.exited) { finish(buffer); return; }
        resize();
        await api.startSession(session.id);
        if (!disposed && !exited) {
          setState("running");
          callbacks.current.onState(session.id, "running");
          if (host.current?.offsetWidth) term.focus();
        }
      } catch (error) {
        if (!disposed) {
          setFailure(error?.message || String(error));
          setState("error");
          callbacks.current.onState(session.id, "error");
          report(error);
        }
      }
    }
    start();
    return () => {
      disposed = true;
      cancelAnimationFrame(frame);
      observer.disconnect();
      offData(); offExit(); write.dispose(); resized.dispose();
      register(session.id, null);
      term.dispose();
      terminal.current = null;
      fitRef.current = null;
    };
  }, [session.id, api, register]);

  useEffect(() => {
    if (!terminal.current) return;
    terminal.current.options.fontSize = settings.fontSize;
    terminal.current.options.cursorBlink = settings.cursorBlink;
    terminal.current.options.scrollback = settings.scrollback;
    const frame = requestAnimationFrame(() => {
      if (host.current?.offsetWidth) fitRef.current?.fit();
    });
    return () => cancelAnimationFrame(frame);
  }, [settings]);

  useEffect(() => {
    if (!visible) return;
    const frame = requestAnimationFrame(() => {
      if (host.current?.offsetWidth) {
        fitRef.current?.fit();
        const term = terminal.current;
        if (term) {
          term.refresh(0, term.rows - 1);
          Promise.resolve(api.resizeSession({ id: session.id, cols: term.cols, rows: term.rows })).catch((error) => callbacks.current.onError(error.message));
        }
      }
    });
    return () => cancelAnimationFrame(frame);
  }, [visible, api, session.id]);

  return <div className={`terminal-view ${visible ? "is-visible" : ""}`} onMouseDown={() => callbacks.current.onFocus(session.id)}>
    <div className="terminal-canvas" ref={host} aria-label={`${session.title}终端`} />
    {state === "opening" && <div className="terminal-loading"><CircleNotch className="spin" size={16} /> 正在启动会话…</div>}
    {state === "error" && <div className="terminal-error"><WarningCircle size={20} /><div><strong>会话启动失败</strong><p>{failure}</p></div></div>}
  </div>;
}
