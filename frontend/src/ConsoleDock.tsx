import { useEffect, useRef, useState } from "react";
import { FitAddon } from "@xterm/addon-fit";
import { Terminal as Xterm } from "@xterm/xterm";
import "@xterm/xterm/css/xterm.css";
import { ChevronDown, ChevronUp, Terminal, X, PanelTopClose } from "lucide-react";
import { dockerAuxConsoleUrl, dockerConsoleUrl, dynamipsConsoleUrl, qemuConsoleUrl, vpcsConsoleUrl } from "./api";
import type { Node } from "./api";
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
  onHide,
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
  const drag = useRef<{x:number;y:number;px:number;py:number}|null>(null);
  const save = (next: typeof position) => { setPosition(next); try { localStorage.setItem("ns.console.float", JSON.stringify({...next,minimized,open:true})); } catch {} };
  const beginDrag = (event: React.PointerEvent) => { drag.current = {x:position.x,y:position.y,px:event.clientX,py:event.clientY}; (event.currentTarget as HTMLElement).setPointerCapture(event.pointerId); };
  const moveDrag = (event: React.PointerEvent) => { if (!drag.current) return; const d=drag.current; save({...position,x:Math.max(0,Math.min(innerWidth-position.width,d.x+event.clientX-d.px)),y:Math.max(0,Math.min(innerHeight-40,d.y+event.clientY-d.py))}); };
  return <section style={{left:position.x,top:position.y,width:minimized?150:position.width,height:minimized?42:position.height}} className={`sim-console-dock ${minimized ? "is-minimized" : ""}`} aria-label="Device consoles">
    <header className="sim-console-tabs">
      <div className="sim-console-tab-list" onPointerDown={beginDrag} onPointerMove={moveDrag} onPointerUp={()=>{drag.current=null; try{localStorage.setItem("ns.console.float",JSON.stringify({...position,minimized,open:true}));}catch{}}} role="tablist" aria-label="Open consoles">
        {targets.map((target) => <div className={`sim-console-tab ${activeNodeId === target.id ? "is-active" : ""}`} key={target.id}>
          <Button variant="ghost" className="sim-console-tab-select" role="tab" aria-selected={activeNodeId === target.id} onClick={() => { onSelect(target.id); onRestore(); }}><span className="sim-console-tab-dot" />{target.nodeName}{target.auxiliary ? " · Aux" : ""}</Button>
          <Button variant="ghost" size="icon-xs" aria-label={`Close ${target.nodeName}${target.auxiliary ? " auxiliary" : ""} console`} onClick={() => onClose(target.id)}><X /></Button>
        </div>)}
      </div>
      <div className="sim-console-dock-actions">
        <span onPointerDown={beginDrag} onPointerMove={moveDrag} onPointerUp={()=>{drag.current=null;}}><Terminal size={14} />Console</span>
        {minimized
          ? <Button variant="ghost" size="icon-sm" onClick={onRestore} aria-label="Restore console dock"><ChevronUp /></Button>
          : <Button variant="ghost" size="icon-sm" onClick={() => { try { localStorage.setItem("ns.console.float", JSON.stringify({...position,minimized:true,open:true})); } catch {} onMinimize(); }} aria-label="Minimize console dock"><ChevronDown /></Button>}
        {!minimized && <Button variant="ghost" size="icon-sm" onClick={() => { try { localStorage.setItem("ns.console.float", JSON.stringify({...position,minimized:false,open:false})); } catch {} onHide(); }} aria-label="Close console dock"><PanelTopClose /></Button>}
      </div>
    </header>
    <div className="sim-console-panes">
      {targets.length ? targets.map((target) => <ConsoleSession key={target.id} target={target} active={target.id === activeNodeId && !minimized} />) : <div className="sim-console-empty">Console · Open a device console to start a session.</div>}
    </div>
    {!minimized && <div className="sim-console-resize" onPointerDown={(e)=>{e.currentTarget.setPointerCapture(e.pointerId); const x=e.clientX,y=e.clientY,p=position; const move=(ev:PointerEvent)=>save({...p,width:Math.max(360,Math.min(innerWidth-p.x,p.width+ev.clientX-x)),height:Math.max(160,Math.min(innerHeight-p.y,p.height+ev.clientY-y))}); const up=()=>{window.removeEventListener("pointermove",move);window.removeEventListener("pointerup",up);};window.addEventListener("pointermove",move);window.addEventListener("pointerup",up);}} />}
  </section>;
}

function ConsoleSession({ target, active }: { target: ConsoleTarget; active: boolean }) {
  const terminalElement = useRef<HTMLDivElement>(null);
  const fitRef = useRef<FitAddon | null>(null);
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
      theme: { background: "#10151d", foreground: "#d6dee9", cursor: "#9db3ff" },
    });
    const fit = new FitAddon();
    fitRef.current = fit;
    terminal.loadAddon(fit);
    terminal.open(terminalElement.current);
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
    terminal.writeln(`${docker ? auxiliary ? "Docker auxiliary" : "Docker" : vpcs ? "VPCS" : dynamips ? "Cisco IOS" : "QEMU serial"} console · ${target.nodeName}`);
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
    };
  }, [auxiliary, docker, dynamips, target.nodeId, target.nodeName, target.projectId, vpcs]);

  useEffect(() => {
    if (!active) return;
    const frame = requestAnimationFrame(() => fitRef.current?.fit());
    return () => cancelAnimationFrame(frame);
  }, [active]);

  return <div className={`sim-console-session ${active ? "is-active" : ""}`} role="region" aria-label={`${target.auxiliary ? "Auxiliary console" : "Console"} for ${target.nodeName}`} aria-hidden={!active}>
    <div className="sim-console-session-meta"><span className="sim-console-tab-dot" />{target.nodeName}<span className="text-muted-foreground">{consoleKind(target.nodeType, target.auxiliary)}</span></div>
    <div ref={terminalElement} className="sim-xterm" />
  </div>;
}

function consoleKind(nodeType: Node["node_type"] | ConsoleTarget["nodeType"], auxiliary = false) {
  return nodeType === "docker" ? auxiliary ? "Docker auxiliary console" : "Docker attached console" : nodeType === "vpcs" ? "VPCS console" : nodeType === "dynamips" ? "Dynamips IOS console" : "QEMU serial console";
}
