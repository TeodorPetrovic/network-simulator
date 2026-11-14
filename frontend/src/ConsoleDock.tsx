import { useEffect, useRef, useState } from "react";
import { clampFloat, popOutWindow, useFloatingFront } from "./floating";
import { FitAddon } from "@xterm/addon-fit";
import { Terminal as Xterm } from "@xterm/xterm";
import "@xterm/xterm/css/xterm.css";
import { ChevronDown, ChevronUp, Terminal, X, ExternalLink } from "lucide-react";
import { dockerAuxConsoleUrl, dockerConsoleUrl, dynamipsConsoleUrl, qemuConsoleUrl, vpcsConsoleUrl } from "./api";
import { Button } from "@/components/ui/button";

export type ConsoleTarget = {
  id: string;
  projectId: string;
  nodeId: string;
  nodeName: string;
  nodeType: "docker" | "qemu" | "vpcs" | "dynamips";
  auxiliary?: boolean;
};

export function dockerResizeControlFrame(rows: number, columns: number): string | null {
  if (!Number.isInteger(rows) || !Number.isInteger(columns) || rows < 1 || columns < 1 || rows > 65_535 || columns > 65_535) return null;
  return JSON.stringify({ type: "resize", rows, columns });
}

export function sendDockerResizeControl(socket: Pick<WebSocket, "readyState" | "send">, rows: number, columns: number): boolean {
  const frame = dockerResizeControlFrame(rows, columns);
  if (!frame || socket.readyState !== WebSocket.OPEN) return false;
  socket.send(frame);
  return true;
}

export function ConsoleDock({
  targets,
  activeNodeId,
  minimized,
  onSelect,
  onMinimize,
  onRestore,
  onClose,
}: {
  targets: ConsoleTarget[];
  activeNodeId: string | null;
  minimized: boolean;
  onSelect: (nodeId: string) => void;
  onMinimize: () => void;
  onRestore: () => void;
  onHide: () => void;
  onClose: (nodeId: string) => void;
}) {
  const defaultPosition = () => { const width = 620, height = 300; const w = typeof window === "undefined" ? 1500 : window.innerWidth; const h = typeof window === "undefined" ? 900 : window.innerHeight; return { x: Math.max(8, w - width - 270), y: Math.max(8, h - height - 48), width, height }; };
  const [position, setPosition] = useState(() => { const d = defaultPosition(); try { const v = JSON.parse(localStorage.getItem("ns.console.float") ?? "{}"); return { x: v.x ?? d.x, y: v.y ?? d.y, width: v.width ?? d.width, height: v.height ?? d.height }; } catch { return d; } });
  useEffect(() => { try { localStorage.setItem("ns.console.float", JSON.stringify({...position,minimized,open:true})); } catch {} }, [position, minimized]);
  const { z, front } = useFloatingFront();
  const resize = useRef<{ x: number; y: number; width: number; height: number; userSelect: string } | null>(null);
  const save = (next: typeof position) => { setPosition(next); try { localStorage.setItem("ns.console.float", JSON.stringify({...next,minimized,open:true})); } catch {} };
  const beginDrag = (event: React.PointerEvent) => {
    if (event.button !== 0) return;
    const start = { x: position.x, y: position.y, px: event.clientX, py: event.clientY };
    let moved = false;
    const move = (pointer: PointerEvent) => {
      if (pointer.buttons === 0) { stop(); return; }
      if (!moved) { if (Math.hypot(pointer.clientX - start.px, pointer.clientY - start.py) < 4) return; moved = true; }
      setPosition((current) => ({ ...current, ...clampFloat(start.x + pointer.clientX - start.px, start.y + pointer.clientY - start.py, minimized ? 150 : current.width) }));
    };
    const stop = () => { window.removeEventListener("pointermove", move); window.removeEventListener("pointerup", stop); window.removeEventListener("pointercancel", stop); };
    window.addEventListener("pointermove", move); window.addEventListener("pointerup", stop); window.addEventListener("pointercancel", stop);
  };
  const PILL = 148;
  const left = minimized ? position.x + position.width - PILL : position.x;
  return <section onPointerDownCapture={front} style={{ left, top: minimized ? position.y + position.height - 36 : position.y, width: minimized ? PILL : position.width, height: minimized ? 36 : position.height, zIndex: z }} className={`fixed z-40 flex flex-col overflow-hidden border border-border bg-card text-card-foreground shadow-xl ${minimized ? "rounded-full" : "rounded-lg"}`} aria-label="Device consoles">
    <header className={`flex select-none touch-none cursor-grab items-stretch justify-between active:cursor-grabbing ${minimized ? "h-9 pl-3 pr-1" : "h-9 border-b border-border"}`} onPointerDown={(event) => { if ((event.target as HTMLElement).closest("button")) return; beginDrag(event); }}>
      {minimized
        ? <div className="flex items-center gap-2 whitespace-nowrap text-xs text-muted-foreground"><Terminal size={14} />Console{targets.length ? ` · ${targets.length}` : ""}</div>
        : <div className="flex min-w-0 overflow-x-auto" role="tablist" aria-label="Open consoles">
          {targets.map((target) => <div className={`flex min-w-28 items-center gap-1 border-r border-border pl-2.5 ${activeNodeId === target.id ? "border-t-2 border-t-primary bg-card" : "border-t-2 border-t-transparent bg-muted/40"}`} key={target.id}>
            <Button variant="ghost" className={`flex h-full min-w-0 flex-1 justify-start gap-2 overflow-hidden px-0 text-xs font-medium hover:bg-transparent ${activeNodeId === target.id ? "text-foreground" : "text-muted-foreground"}`} role="tab" aria-selected={activeNodeId === target.id} onClick={() => { onSelect(target.id); onRestore(); }}><span className="size-1.5 shrink-0 rounded-full bg-success" /><span className="truncate">{target.nodeName}{target.auxiliary ? " · Aux" : ""}</span></Button>
            <Button variant="ghost" size="icon-xs" aria-label={`Close ${target.nodeName}${target.auxiliary ? " auxiliary" : ""} console`} onClick={() => onClose(target.id)}><X /></Button>
          </div>)}
        </div>}
      <div className="flex items-center gap-0.5 px-1">
        {!minimized && activeNodeId && <Button variant="ghost" size="icon-sm" aria-label="Pop out console" title="Open this console in a new window" onClick={() => { const t = targets.find((item) => item.id === activeNodeId); if (!t) return; const q = new URLSearchParams({ projectId: t.projectId, nodeId: t.nodeId, nodeName: t.nodeName, nodeType: t.nodeType, ...(t.auxiliary ? { auxiliary: "1" } : {}) }); popOutWindow(`/console?${q}`, 820, 520, t.nodeName); onClose(t.id); }}><ExternalLink /></Button>}
        {minimized
          ? <Button variant="ghost" size="icon-sm" className="rounded-full" onClick={onRestore} aria-label="Restore console dock"><ChevronUp /></Button>
          : <Button variant="ghost" size="icon-sm" onClick={() => { try { localStorage.setItem("ns.console.float", JSON.stringify({ ...position, minimized: true, open: true })); } catch {} onMinimize(); }} aria-label="Minimize console dock"><ChevronDown /></Button>}
      </div>
    </header>
    <div className={`relative flex min-h-0 flex-1 ${minimized ? "hidden" : ""}`}>
      {targets.length ? targets.map((target) => <ConsoleSession key={target.id} target={target} active={target.id === activeNodeId && !minimized} />) : <div className="grid min-h-0 flex-1 place-items-center text-[11px] text-muted-foreground">Console · Open a device console to start a session.</div>}
    </div>
    {!minimized && <div className="pointer-events-auto absolute right-0 bottom-0 z-20 size-4.5 cursor-nwse-resize touch-none bg-[linear-gradient(135deg,transparent_50%,var(--muted-foreground)_50%,var(--muted-foreground)_58%,transparent_58%,transparent_70%,var(--muted-foreground)_70%,var(--muted-foreground)_78%,transparent_78%)]" role="separator" aria-label="Resize console dock" onPointerDown={(e) => { e.preventDefault(); e.stopPropagation(); const start = { x: e.clientX, y: e.clientY, width: position.width, height: position.height, userSelect: document.body.style.userSelect }; resize.current = start; document.body.style.userSelect = "none"; const move = (event: PointerEvent) => { if (!resize.current || event.buttons === 0) return; save({ ...position, width: Math.max(360, Math.min(window.innerWidth - position.x, start.width + event.clientX - start.x)), height: Math.max(160, Math.min(window.innerHeight - position.y, start.height + event.clientY - start.y)) }); }; const stop = () => { window.removeEventListener("pointermove", move); window.removeEventListener("pointerup", stop); window.removeEventListener("pointercancel", stop); if (resize.current) document.body.style.userSelect = resize.current.userSelect; resize.current = null; }; window.addEventListener("pointermove", move); window.addEventListener("pointerup", stop); window.addEventListener("pointercancel", stop); }} />}
  </section>;
}

export function ConsoleSession({ target, active }: { target: ConsoleTarget; active: boolean }) {
  const terminalElement = useRef<HTMLDivElement>(null);
  const fitRef = useRef<FitAddon | null>(null);
  const terminalRef = useRef<Xterm | null>(null);
  const activeRef = useRef(active);
  activeRef.current = active;
  const docker = target.nodeType === "docker";
  const auxiliary = Boolean(target.auxiliary);
  const vpcs = target.nodeType === "vpcs";
  const dynamips = target.nodeType === "dynamips";

  useEffect(() => {
    if (!terminalElement.current) return;
    const terminal = new Xterm({
      convertEol: true,
      cursorBlink: true,
      fontFamily: "ui-monospace, SFMono-Regular, Menlo, monospace",
      fontSize: 12,
      theme: terminalTheme(),
    });
    const fit = new FitAddon();
    fitRef.current = fit;
    terminal.loadAddon(fit);
    terminal.open(terminalElement.current);
    terminalRef.current = terminal;
    if (activeRef.current) terminal.focus();
    let socket: WebSocket | null = null;
    let cancelled = false;
    let lastResizeFrame: string | null = null;
    const sendResize = () => {
      if (!docker || auxiliary) return;
      const frame = dockerResizeControlFrame(terminal.rows, terminal.cols);
      if (socket && frame && frame !== lastResizeFrame && sendDockerResizeControl(socket, terminal.rows, terminal.cols)) lastResizeFrame = frame;
    };
    const fitVisibleTerminal = () => {
      if (terminalElement.current?.offsetWidth && terminalElement.current.offsetHeight) {
        fit.fit();
        sendResize();
      }
    };
    const resize = terminal.onResize(sendResize);
    requestAnimationFrame(fitVisibleTerminal);
    const socketUrl = docker
      ? auxiliary ? dockerAuxConsoleUrl(target.projectId, target.nodeId) : dockerConsoleUrl(target.projectId, target.nodeId)
      : vpcs ? vpcsConsoleUrl(target.projectId, target.nodeId) : dynamips ? dynamipsConsoleUrl(target.projectId, target.nodeId) : qemuConsoleUrl(target.projectId, target.nodeId);
    void Promise.resolve(socketUrl).then((url) => {
      if (cancelled) return;
      socket = new WebSocket(url);
      socket.binaryType = "arraybuffer";
      socket.onopen = () => {
        terminal.writeln(docker ? auxiliary ? "Attached to Docker auxiliary console." : "Attached to Docker container terminal." : vpcs ? "Connected to VPCS console." : dynamips ? "Connected to Cisco IOS console." : "Connected to QEMU serial port.");
        sendResize();
      };
      socket.onmessage = (event) => terminal.write(typeof event.data === "string" ? event.data : new Uint8Array(event.data as ArrayBuffer));
      socket.onerror = () => terminal.writeln("\r\nConsole connection failed.");
      socket.onclose = () => terminal.writeln("\r\nConsole disconnected.");
    }).catch(() => terminal.writeln("\r\nConsole connection failed."));
    const input = terminal.onData((data) => { if (socket?.readyState === WebSocket.OPEN) socket.send(data); });
    const observer = typeof ResizeObserver === "undefined" ? null : new ResizeObserver(fitVisibleTerminal);
    observer?.observe(terminalElement.current);
    window.addEventListener("resize", fitVisibleTerminal);
    return () => {
      cancelled = true;
      window.removeEventListener("resize", fitVisibleTerminal);
      observer?.disconnect();
      input.dispose();
      resize.dispose();
      if (socket) {
        socket.onopen = null;
        socket.onmessage = null;
        socket.onerror = null;
        socket.onclose = null;
        socket.close();
      }
      terminal.dispose();
      fitRef.current = null;
      terminalRef.current = null;
    };
  }, [auxiliary, docker, dynamips, target.nodeId, target.nodeName, target.projectId, vpcs]);

  useEffect(() => {
    if (!active) return;
    const frame = requestAnimationFrame(() => { fitRef.current?.fit(); terminalRef.current?.focus(); });
    return () => cancelAnimationFrame(frame);
  }, [active]);

  return <div className={`${active ? "flex" : "hidden"} h-full flex-col`} role="region" aria-label={`${target.auxiliary ? "Auxiliary console" : "Console"} for ${target.nodeName}`} aria-hidden={!active}>
    <div ref={terminalElement} className="min-h-0 w-full flex-1 px-2.5 py-1.5 [&_.xterm]:h-full [&_.xterm-viewport]:!overflow-y-auto" />
  </div>;
}

function terminalTheme() {
  const css = getComputedStyle(document.documentElement);
  const color = (name: string, fallback: string) => css.getPropertyValue(name).trim() || fallback;
  return { background: color("--card", "#10151d"), foreground: color("--foreground", "#d6dee9"), cursor: color("--primary", "#9db3ff"), selectionBackground: color("--accent", "#2a3550") };
}
