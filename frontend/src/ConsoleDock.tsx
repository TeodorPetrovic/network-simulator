import { useEffect, useRef } from "react";
import { FitAddon } from "@xterm/addon-fit";
import { Terminal as Xterm } from "@xterm/xterm";
import "@xterm/xterm/css/xterm.css";
import { ChevronDown, ChevronUp, Terminal, X } from "lucide-react";
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
  onClose,
}: {
  targets: ConsoleTarget[];
  activeNodeId: string | null;
  minimized: boolean;
  onSelect: (nodeId: string) => void;
  onMinimize: () => void;
  onRestore: () => void;
  onClose: (nodeId: string) => void;
}) {
  if (targets.length === 0) return null;
  return <section className={`sim-console-dock ${minimized ? "is-minimized" : ""}`} aria-label="Device consoles">
    <header className="sim-console-tabs">
      <div className="sim-console-tab-list" role="tablist" aria-label="Open consoles">
        {targets.map((target) => <div className={`sim-console-tab ${activeNodeId === target.id ? "is-active" : ""}`} key={target.id}>
          <Button variant="ghost" className="sim-console-tab-select" role="tab" aria-selected={activeNodeId === target.id} onClick={() => { onSelect(target.id); onRestore(); }}><span className="sim-console-tab-dot" />{target.nodeName}{target.auxiliary ? " · Aux" : ""}</Button>
          <Button variant="ghost" size="icon-xs" aria-label={`Close ${target.nodeName}${target.auxiliary ? " auxiliary" : ""} console`} onClick={() => onClose(target.id)}><X /></Button>
        </div>)}
      </div>
      <div className="sim-console-dock-actions">
        <span><Terminal size={14} />Console dock</span>
        {minimized
          ? <Button variant="ghost" size="icon-sm" onClick={onRestore} aria-label="Restore console dock"><ChevronUp /></Button>
          : <Button variant="ghost" size="icon-sm" onClick={onMinimize} aria-label="Minimize console dock"><ChevronDown /></Button>}
      </div>
    </header>
    <div className="sim-console-panes">
      {targets.map((target) => <ConsoleSession key={target.id} target={target} active={target.id === activeNodeId && !minimized} />)}
    </div>
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
    const socket = new WebSocket(docker
      ? auxiliary ? dockerAuxConsoleUrl(target.projectId, target.nodeId) : dockerConsoleUrl(target.projectId, target.nodeId)
      : vpcs ? vpcsConsoleUrl(target.projectId, target.nodeId) : dynamips ? dynamipsConsoleUrl(target.projectId, target.nodeId) : qemuConsoleUrl(target.projectId, target.nodeId));
    socket.binaryType = "arraybuffer";
    let lastResizeFrame: string | null = null;
    const sendResize = () => {
      if (!docker || auxiliary) return;
      const frame = dockerResizeControlFrame(terminal.rows, terminal.cols);
      if (frame && frame !== lastResizeFrame && sendDockerResizeControl(socket, terminal.rows, terminal.cols)) lastResizeFrame = frame;
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
    socket.onopen = () => {
      terminal.writeln(docker ? auxiliary ? "Attached to Docker auxiliary console." : "Attached to Docker container terminal." : vpcs ? "Connected to VPCS console." : dynamips ? "Connected to Cisco IOS console." : "Connected to QEMU serial port.");
      sendResize();
    };
    socket.onmessage = (event) => terminal.write(typeof event.data === "string" ? event.data : new Uint8Array(event.data as ArrayBuffer));
    socket.onerror = () => terminal.writeln("\r\nConsole connection failed.");
    socket.onclose = () => terminal.writeln("\r\nConsole disconnected.");
    const input = terminal.onData((data) => { if (socket.readyState === WebSocket.OPEN) socket.send(data); });
    const observer = typeof ResizeObserver === "undefined" ? null : new ResizeObserver(fitVisibleTerminal);
    observer?.observe(terminalElement.current);
    window.addEventListener("resize", fitVisibleTerminal);
    return () => {
      window.removeEventListener("resize", fitVisibleTerminal);
      observer?.disconnect();
      input.dispose();
      resize.dispose();
      socket.onopen = null;
      socket.onmessage = null;
      socket.onerror = null;
      socket.onclose = null;
      socket.close();
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
