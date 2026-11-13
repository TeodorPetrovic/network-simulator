import { useEffect, useRef, useState } from "react";
import RFB from "@novnc/novnc";
import { X } from "lucide-react";
import { Button } from "@/components/ui/button";
import { vncConsoleUrl } from "./api";
import { clampFloat, useFloatingFront } from "./floating";

type Props = { runtime: "qemu" | "docker"; projectId: string; nodeId: string; nodeName: string; onClose: () => void };

export default function VncConsole({ runtime, projectId, nodeId, nodeName, onClose }: Props) {
  const screen = useRef<HTMLDivElement>(null);
  const [status, setStatus] = useState("Connecting to console…");
  const [rect, setRect] = useState(() => {
    const width = Math.min(1100, Math.round(window.innerWidth * 0.8));
    const height = Math.min(760, Math.round(window.innerHeight * 0.8));
    return { x: Math.round((window.innerWidth - width) / 2), y: Math.round((window.innerHeight - height) / 2), width, height };
  });
  const { z, front } = useFloatingFront();
  // Drag by the title bar, resize from the bottom-right corner; both follow the pointer until release.
  const track = (event: React.PointerEvent, apply: (dx: number, dy: number) => void) => {
    if (event.button !== 0) return;
    event.preventDefault();
    const startX = event.clientX, startY = event.clientY;
    const move = (pointer: PointerEvent) => { if (pointer.buttons === 0) stop(); else apply(pointer.clientX - startX, pointer.clientY - startY); };
    const stop = () => { window.removeEventListener("pointermove", move); window.removeEventListener("pointerup", stop); window.removeEventListener("pointercancel", stop); };
    window.addEventListener("pointermove", move); window.addEventListener("pointerup", stop); window.addEventListener("pointercancel", stop);
  };
  const beginDrag = (event: React.PointerEvent) => {
    if ((event.target as HTMLElement).closest("button")) return;
    const start = rect;
    track(event, (dx, dy) => setRect({ ...start, ...clampFloat(start.x + dx, start.y + dy, start.width) }));
  };
  const beginResize = (event: React.PointerEvent) => {
    event.stopPropagation();
    const start = rect;
    track(event, (dx, dy) => setRect({ ...start, width: Math.max(360, Math.min(window.innerWidth - start.x, start.width + dx)), height: Math.max(240, Math.min(window.innerHeight - start.y, start.height + dy)) }));
  };
  useEffect(() => {
    let client: InstanceType<typeof RFB> | undefined;
    let alive = true;
    void vncConsoleUrl(runtime, projectId, nodeId).then((url) => {
      if (!alive || !screen.current) return;
      client = new RFB(screen.current, url, { wsProtocols: ["binary"] });
      client.scaleViewport = true;
      client.resizeSession = true;
      client.addEventListener("connect", () => setStatus("Connected"));
      client.addEventListener("disconnect", (event) => setStatus((event as CustomEvent<{ clean: boolean }>).detail.clean ? "Disconnected" : "Connection lost"));
    }).catch((error: unknown) => setStatus(error instanceof Error ? error.message : "Could not connect to VNC console"));
    return () => { alive = false; client?.disconnect(); };
  }, [runtime, projectId, nodeId]);
  return <section onPointerDownCapture={front} style={{ left: rect.x, top: rect.y, width: rect.width, height: rect.height, zIndex: z }} className="fixed flex flex-col overflow-hidden rounded-lg border border-border bg-card text-card-foreground shadow-2xl" aria-label={`${nodeName} VNC console`}>
    <header className="flex h-9 shrink-0 cursor-grab touch-none select-none items-center gap-3 border-b border-border pl-3 pr-1 active:cursor-grabbing" onPointerDown={beginDrag}><strong className="text-sm">{nodeName} · VNC</strong><span className="flex-1 truncate text-xs text-muted-foreground">{status}</span><Button variant="ghost" size="icon-sm" aria-label="Close VNC console" onClick={onClose}><X size={16} /></Button></header>
    <div className="min-h-0 flex-1 overflow-hidden bg-black [&_canvas]:block" ref={screen} />
    <div className="absolute right-0 bottom-0 size-4.5 cursor-nwse-resize touch-none bg-[linear-gradient(135deg,transparent_50%,var(--muted-foreground)_50%,var(--muted-foreground)_58%,transparent_58%,transparent_70%,var(--muted-foreground)_70%,var(--muted-foreground)_78%,transparent_78%)]" role="separator" aria-label="Resize VNC console" onPointerDown={beginResize} />
  </section>;
}
