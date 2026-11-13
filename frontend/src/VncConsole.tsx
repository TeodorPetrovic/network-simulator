import { useEffect, useRef, useState } from "react";
import RFB from "@novnc/novnc";
import { X } from "lucide-react";
import { Button } from "@/components/ui/button";
import { vncConsoleUrl } from "./api";

type Props = { runtime: "qemu" | "docker"; projectId: string; nodeId: string; nodeName: string; onClose: () => void };

export default function VncConsole({ runtime, projectId, nodeId, nodeName, onClose }: Props) {
  const screen = useRef<HTMLDivElement>(null);
  const [status, setStatus] = useState("Connecting to console…");
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
  return <section className="fixed inset-[7vh_7vw] z-[60] flex flex-col overflow-hidden rounded-lg border border-border bg-card text-card-foreground shadow-2xl max-[700px]:inset-[4vh_3vw]" role="dialog" aria-label={`${nodeName} VNC console`}>
    <header className="flex min-h-[42px] items-center gap-3.5 border-b border-border px-3"><strong>{nodeName} · VNC</strong><span className="flex-1 text-xs text-muted-foreground">{status}</span><Button variant="ghost" size="icon-sm" aria-label="Close VNC console" onClick={onClose}><X size={16} /></Button></header>
    <div className="min-h-0 flex-1 overflow-hidden [&_canvas]:block" ref={screen} />
  </section>;
}
