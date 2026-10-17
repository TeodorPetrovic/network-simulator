import { useEffect, useRef } from "react";
import { FitAddon } from "@xterm/addon-fit";
import { Terminal as Xterm } from "@xterm/xterm";
import "@xterm/xterm/css/xterm.css";
import { ChevronDown, ChevronUp, Terminal, X } from "lucide-react";
import { dockerConsoleUrl, dynamipsConsoleUrl, qemuConsoleUrl, vpcsConsoleUrl } from "./api";
import type { Node } from "./api";
import { Button } from "@/components/ui/button";

export type ConsoleTarget = {
  projectId: string;
  nodeId: string;
  nodeName: string;
  nodeType: "docker" | "qemu" | "vpcs" | "dynamips";
};

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
        {targets.map((target) => <div className={`sim-console-tab ${activeNodeId === target.nodeId ? "is-active" : ""}`} key={target.nodeId}>
          <Button variant="ghost" className="sim-console-tab-select" role="tab" aria-selected={activeNodeId === target.nodeId} onClick={() => { onSelect(target.nodeId); onRestore(); }}><span className="sim-console-tab-dot" />{target.nodeName}</Button>
          <Button variant="ghost" size="icon-xs" aria-label={`Close ${target.nodeName} console`} onClick={() => onClose(target.nodeId)}><X /></Button>
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
      {targets.map((target) => <ConsoleSession key={target.nodeId} target={target} active={target.nodeId === activeNodeId && !minimized} />)}
    </div>
  </section>;
}

function ConsoleSession({ target, active }: { target: ConsoleTarget; active: boolean }) {
  const terminalElement = useRef<HTMLDivElement>(null);
  const fitRef = useRef<FitAddon | null>(null);
  const docker = target.nodeType === "docker";
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
    const fitVisibleTerminal = () => {
      if (terminalElement.current?.offsetWidth && terminalElement.current.offsetHeight) fit.fit();
    };
    requestAnimationFrame(fitVisibleTerminal);
    terminal.writeln(`${docker ? "Docker" : vpcs ? "VPCS" : dynamips ? "Cisco IOS" : "QEMU serial"} console · ${target.nodeName}`);
    const socket = new WebSocket(docker
      ? dockerConsoleUrl(target.projectId, target.nodeId)
      : vpcs ? vpcsConsoleUrl(target.projectId, target.nodeId) : dynamips ? dynamipsConsoleUrl(target.projectId, target.nodeId) : qemuConsoleUrl(target.projectId, target.nodeId));
    socket.binaryType = "arraybuffer";
    socket.onopen = () => terminal.writeln(docker ? "Attached to Docker container terminal." : vpcs ? "Connected to VPCS console." : dynamips ? "Connected to Cisco IOS console." : "Connected to QEMU serial port.");
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
      socket.onopen = null;
      socket.onmessage = null;
      socket.onerror = null;
      socket.onclose = null;
      socket.close();
      terminal.dispose();
      fitRef.current = null;
    };
  }, [docker, dynamips, target.nodeId, target.nodeName, target.projectId, vpcs]);

  useEffect(() => {
    if (!active) return;
    const frame = requestAnimationFrame(() => fitRef.current?.fit());
    return () => cancelAnimationFrame(frame);
  }, [active]);

  return <div className={`sim-console-session ${active ? "is-active" : ""}`} role="region" aria-label={`Console for ${target.nodeName}`} aria-hidden={!active}>
    <div className="sim-console-session-meta"><span className="sim-console-tab-dot" />{target.nodeName}<span className="text-muted-foreground">{consoleKind(target.nodeType)}</span></div>
    <div ref={terminalElement} className="sim-xterm" />
  </div>;
}

function consoleKind(nodeType: Node["node_type"] | ConsoleTarget["nodeType"]) {
  return nodeType === "docker" ? "Docker attached console" : nodeType === "vpcs" ? "VPCS console" : nodeType === "dynamips" ? "Dynamips IOS console" : "QEMU serial console";
}
