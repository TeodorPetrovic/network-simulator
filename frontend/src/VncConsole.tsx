import { useEffect, useRef, useState } from "react";
import RFB from "@novnc/novnc";
import { X } from "lucide-react";
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
  return <section className="sim-vnc-window" role="dialog" aria-label={`${nodeName} VNC console`}>
    <header><strong>{nodeName} · VNC</strong><span>{status}</span><button type="button" aria-label="Close VNC console" onClick={onClose}><X size={16} /></button></header>
    <div className="sim-vnc-screen" ref={screen} />
  </section>;
}
