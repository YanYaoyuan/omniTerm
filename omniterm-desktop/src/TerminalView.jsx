import { useEffect, useRef, useState } from "react";
import { Terminal } from "@xterm/xterm";
import { FitAddon } from "@xterm/addon-fit";
import { SearchAddon } from "@xterm/addon-search";
import { Unicode11Addon } from "@xterm/addon-unicode11";
import { WebLinksAddon } from "@xterm/addon-web-links";
import { WebglAddon } from "@xterm/addon-webgl";
import { CircleNotch, WarningCircle } from "@phosphor-icons/react";
import { isAppShortcut } from "./shortcuts.js";
import { terminalTheme, themeFor } from "./themes.js";
import { cwdFromOsc7, quotePath } from "./terminal-utils.js";

export const FONT_STACKS = {
  system: '"SFMono-Regular", Menlo, Monaco, Consolas, "Liberation Mono", monospace',
  jetbrains: '"JetBrains Mono", "SFMono-Regular", Menlo, Consolas, monospace',
  fira: '"Fira Code", "SFMono-Regular", Menlo, Consolas, monospace',
  cascadia: '"Cascadia Code", "Cascadia Mono", Consolas, "SFMono-Regular", monospace',
  sarasa: '"Sarasa Mono SC", "Sarasa Term SC", "SFMono-Regular", Menlo, Consolas, monospace',
};

const searchDecorations = {
  matchBackground: "#dec38d55", matchOverviewRuler: "#dec38d",
  activeMatchBackground: "#8db4ffaa", activeMatchColorOverviewRuler: "#8db4ff",
};

function loadWebgl(term) {
  try {
    const webgl = new WebglAddon();
    webgl.onContextLoss(() => webgl.dispose());
    term.loadAddon(webgl);
    return webgl;
  } catch (error) {
    // Fall back to the DOM renderer when WebGL2 is unavailable.
    console.warn("WebGL renderer unavailable:", error);
    return null;
  }
}

export function TerminalView({ session, visible, selected = false, focused = false, dimmed = false, style, settings, clear = false, platform, hostname, api, register, onState, onError, onFocus, onInput, onCwd, onTitle }) {
  const host = useRef(null);
  const terminal = useRef(null);
  const fitRef = useRef(null);
  const webglRef = useRef(null);
  const settingsRef = useRef(settings);
  const callbacks = useRef({ onState, onError, onFocus, onInput, onCwd, onTitle });
  const selectedRef = useRef(selected);
  const hostnameRef = useRef(hostname);
  const [state, setState] = useState("opening");
  const [failure, setFailure] = useState("");
  const [dragging, setDragging] = useState(false);
  callbacks.current = { onState, onError, onFocus, onInput, onCwd, onTitle };
  settingsRef.current = settings;
  selectedRef.current = selected;
  hostnameRef.current = hostname;

  useEffect(() => {
    let disposed = false;
    let exited = false;
    let frame;
    const term = new Terminal({
      cursorBlink: settings.cursorBlink,
      cursorStyle: settings.cursorStyle,
      fontSize: settings.fontSize,
      fontFamily: FONT_STACKS[settings.fontFamily] || FONT_STACKS.system,
      lineHeight: settings.lineHeight,
      letterSpacing: 0,
      scrollback: settings.scrollback,
      // Required by the Unicode 11 width provider.
      allowProposedApi: true,
      macOptionIsMeta: settings.optionAsMeta,
      macOptionClickForcesSelection: true,
      rightClickSelectsWord: platform === "darwin",
      smoothScrollDuration: 0,
      allowTransparency: clear,
      theme: terminalTheme(themeFor(settings.theme), clear),
    });
    const fit = new FitAddon();
    const search = new SearchAddon();
    const unicode = new Unicode11Addon();
    term.loadAddon(fit);
    term.loadAddon(search);
    term.loadAddon(unicode);
    term.unicode.activeVersion = "11";
    term.loadAddon(new WebLinksAddon((event, uri) => {
      // Like iTerm, links open on Cmd/Ctrl+click so plain clicks still select text.
      if (!(event.metaKey || event.ctrlKey)) return;
      Promise.resolve(api.openExternal(uri)).catch(report);
    }));
    term.open(host.current);
    if (settings.gpu) webglRef.current = loadWebgl(term);
    terminal.current = term;
    fitRef.current = fit;

    function report(error) {
      if (!disposed && error) callbacks.current.onError(error?.message || String(error));
    }
    const write = term.onData((data) => {
      if (exited) return;
      if (callbacks.current.onInput) callbacks.current.onInput(session.id, data);
      else Promise.resolve(api.writeSession({ id: session.id, data })).catch(report);
    });
    const resized = term.onResize(({ cols, rows }) => {
      Promise.resolve(api.resizeSession({ id: session.id, cols, rows })).catch(report);
    });
    const titled = term.onTitleChange((title) => callbacks.current.onTitle?.(session.id, title.slice(0, 120)));
    const selected = term.onSelectionChange(() => {
      if (!settingsRef.current.copyOnSelect || !term.hasSelection()) return;
      navigator.clipboard.writeText(term.getSelection()).catch(() => {});
    });
    const osc7 = term.parser.registerOscHandler(7, (data) => {
      const cwd = cwdFromOsc7(data, platform, hostnameRef.current);
      if (cwd) callbacks.current.onCwd?.(session.id, cwd);
      return true;
    });
    // Keyboard focus is the source of truth: whatever gives this terminal focus,
    // selection, the breadcrumb and the close target must follow it.
    const focused = () => callbacks.current.onFocus(session.id);
    term.textarea?.addEventListener("focus", focused);
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
      find: (value, previous = false, options = {}) => {
        if (!value) { search.clearDecorations(); term.clearSelection(); return true; }
        const query = { ...options, decorations: searchDecorations };
        return previous ? search.findPrevious(value, query) : search.findNext(value, query);
      },
      clearSearch: () => { search.clearDecorations(); term.clearSelection(); },
      getSelection: () => term.getSelection(),
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
      // Uses bracketed paste when the program enabled it, so multi-line text
      // is not executed line by line.
      pasteText: (value) => {
        if (exited) throw new Error("该会话已结束。");
        term.paste(value);
      },
      selectAll: () => term.selectAll(),
      scrollToBottom: () => term.scrollToBottom(),
      clear: () => { term.clear(); term.focus(); },
      isExited: () => exited,
    };
    register(session.id, controls);
    term.attachCustomKeyEventHandler((event) => {
      if (event.type !== "keydown") return true;
      if (isAppShortcut(event, platform)) return false;
      const key = event.key.toLowerCase();
      const clipboardShortcut = platform === "darwin" ? event.metaKey && !event.ctrlKey : event.ctrlKey && event.shiftKey;
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
          // Panes finish starting in any order; only the selected one takes focus.
          if (selectedRef.current && host.current?.offsetWidth) term.focus();
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
      term.textarea?.removeEventListener("focus", focused);
      offData(); offExit(); write.dispose(); resized.dispose(); titled.dispose(); selected.dispose(); osc7.dispose();
      register(session.id, null);
      webglRef.current = null;
      term.dispose();
      terminal.current = null;
      fitRef.current = null;
    };
  }, [session.id, api, register, platform]);

  useEffect(() => {
    const term = terminal.current;
    if (!term) return;
    term.options.fontSize = settings.fontSize;
    term.options.fontFamily = FONT_STACKS[settings.fontFamily] || FONT_STACKS.system;
    term.options.lineHeight = settings.lineHeight;
    term.options.cursorBlink = settings.cursorBlink;
    term.options.cursorStyle = settings.cursorStyle;
    term.options.scrollback = settings.scrollback;
    term.options.macOptionIsMeta = settings.optionAsMeta;
    term.options.theme = terminalTheme(themeFor(settings.theme), clear);
    if (term.options.allowTransparency !== clear) {
      // The WebGL glyph atlas is built for one mode and must be rebuilt for the other.
      term.options.allowTransparency = clear;
      webglRef.current?.dispose();
      webglRef.current = null;
    }
    if (settings.gpu && !webglRef.current) webglRef.current = loadWebgl(term);
    else if (!settings.gpu && webglRef.current) { webglRef.current.dispose(); webglRef.current = null; }
    const frame = requestAnimationFrame(() => {
      if (host.current?.offsetWidth) fitRef.current?.fit();
    });
    return () => cancelAnimationFrame(frame);
  }, [settings, clear]);

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

  const drop = (event) => {
    event.preventDefault();
    setDragging(false);
    const files = [...(event.dataTransfer?.files || [])];
    const paths = files.map((file) => api.pathForFile?.(file)).filter(Boolean);
    if (!paths.length || state !== "running") return;
    callbacks.current.onInput?.(session.id, paths.map((filename) => quotePath(filename, platform, session.powershell)).join(" ") + " ");
    terminal.current?.focus();
  };

  return <div
    className={`terminal-view ${visible ? "is-visible" : ""} ${focused ? "is-focused" : ""} ${dimmed ? "is-dimmed" : ""} ${dragging ? "is-dragging" : ""}`}
    style={style}
    onMouseDown={() => callbacks.current.onFocus(session.id)}
    onDragOver={(event) => { if (event.dataTransfer?.types?.includes("Files")) { event.preventDefault(); setDragging(true); } }}
    onDragLeave={() => setDragging(false)}
    onDrop={drop}
  >
    <div className="terminal-canvas" ref={host} aria-label={`${session.title}终端`} />
    {state === "opening" && <div className="terminal-loading"><CircleNotch className="spin" size={16} /> 正在启动会话…</div>}
    {state === "error" && <div className="terminal-error"><WarningCircle size={20} /><div><strong>会话启动失败</strong><p>{failure}</p></div></div>}
  </div>;
}
