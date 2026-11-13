import { useEffect, useMemo, useRef, useState } from "react";
import { ChevronDown, ChevronUp, ExternalLink, LoaderCircle, RotateCw, X } from "lucide-react";
import { Button } from "@/components/ui/button";
import { api, serverOriginOrSelf, webWiresharkClientUrl } from "../api";
import { clampFloat, popOutWindow, useFloatingFront } from "../floating";

type Props = { projectId: string; linkId: string; wsPath: string; title: string; onClose: () => void };

export function WebWiresharkWindow({ projectId, linkId, wsPath, title, onClose }: Props) {
  const { z, front } = useFloatingFront();
  const [interacting, setInteracting] = useState(false);
  const [minimized, setMinimized] = useState(false);
  const [position, setPosition] = useState({ x: 24, y: 24, width: 960, height: 680 });
  const [frameKey, setFrameKey] = useState(0);
  const [loaded, setLoaded] = useState(false);
  const [error, setError] = useState("");
  const frameRef = useRef<HTMLIFrameElement>(null);
  const source = useMemo(() => webWiresharkClientUrl(wsPath), [wsPath, frameKey]);
  useEffect(() => {
    if (loaded || error) return;
    const timer = window.setTimeout(() => setError("Web Wireshark did not connect within 60 seconds."), 60_000);
    return () => window.clearTimeout(timer);
  }, [frameKey, loaded, error]);
  useEffect(() => {
    const connected = (event: MessageEvent) => {
      if (event.source !== frameRef.current?.contentWindow || event.origin !== serverOriginOrSelf() || event.data?.type !== "xpra-connected") return;
      setLoaded(true);
      setError("");
    };
    window.addEventListener("message", connected);
    return () => window.removeEventListener("message", connected);
  }, []);
  async function restart() {
    setError(""); setLoaded(false);
    try { await api.restartWebWireshark(projectId, linkId); setFrameKey((value) => value + 1); }
    catch (reason) { setError(reason instanceof Error ? reason.message : "Could not restart Wireshark."); }
  }
  return <section className={`fixed z-[45] flex min-h-[280px] min-w-[420px] flex-col overflow-hidden rounded-lg border border-border bg-card shadow-[0_12px_36px_oklch(0_0_0/0.45)] ${minimized ? "min-w-[240px] min-h-[42px]" : ""}`} style={{ left: position.x, top: position.y, width: minimized ? 340 : position.width, height: minimized ? 42 : position.height, zIndex: z }} onPointerDownCapture={front} aria-label={title}>
    <header className="flex h-[42px] min-h-[42px] cursor-move items-center gap-2 bg-muted px-2 font-semibold [&>span]:flex-1" onPointerDown={(event) => {
      if ((event.target as HTMLElement).closest("button")) return;
      const x = event.clientX, y = event.clientY, startX = position.x, startY = position.y;
      const move = (pointer: PointerEvent) => setPosition((current) => ({ ...current, ...clampFloat(startX + pointer.clientX - x, startY + pointer.clientY - y, current.width) }));
      const stop = () => { setInteracting(false); window.removeEventListener("pointermove", move); window.removeEventListener("pointerup", stop); };
      setInteracting(true);
      window.addEventListener("pointermove", move); window.addEventListener("pointerup", stop);
    }}>{title}<span />{!minimized && <Button size="sm" variant="ghost" onClick={() => void restart()}><RotateCw />Restart Wireshark</Button>}<Button size="icon-xs" variant="ghost" aria-label="Pop out Web Wireshark" title="Open in a new window" onClick={() => { popOutWindow(webWiresharkClientUrl(wsPath), 1100, 760, "Web Wireshark"); onClose(); }}><ExternalLink /></Button><Button size="icon-xs" variant="ghost" aria-label={minimized ? "Restore Web Wireshark" : "Minimize Web Wireshark"} onClick={() => setMinimized((value) => !value)}>{minimized ? <ChevronUp /> : <ChevronDown />}</Button><Button size="icon-xs" variant="ghost" aria-label="Close Web Wireshark" onClick={onClose}><X /></Button></header>
    {!minimized && <div className={`relative flex min-h-0 flex-1 ${interacting ? "[&_iframe]:pointer-events-none" : ""}`}>{!loaded && <div className="absolute inset-0 z-[1] flex items-center justify-center gap-2.5 bg-background/90 text-foreground" role="status"><LoaderCircle className="animate-spin" />{error || "Connecting to Web Wireshark…"}{error && <Button size="sm" onClick={() => { setError(""); setLoaded(false); setFrameKey((value) => value + 1); }}>Retry</Button>}</div>}<iframe className="h-full w-full border-0" ref={frameRef} key={frameKey} title="Web Wireshark" src={source} />{!error && !loaded && <span className="sr-only">Waiting for the Wireshark session to connect</span>}</div>}
    {!minimized && <div className="absolute bottom-0 right-0 z-[3] h-[22px] w-[22px] cursor-nwse-resize touch-none opacity-70 hover:opacity-100 bg-[linear-gradient(135deg,transparent_0_55%,var(--muted-foreground)_55%_62%,transparent_62%_72%,var(--muted-foreground)_72%_79%,transparent_79%)]" onPointerDown={(event) => { event.preventDefault(); const x = event.clientX, y = event.clientY, width = position.width, height = position.height; const move = (pointer: PointerEvent) => setPosition((current) => ({ ...current, width: Math.max(420, Math.min(innerWidth - current.x, width + pointer.clientX - x)), height: Math.max(280, Math.min(innerHeight - current.y, height + pointer.clientY - y)) })); const stop = () => { setInteracting(false); window.removeEventListener("pointermove", move); window.removeEventListener("pointerup", stop); }; setInteracting(true); window.addEventListener("pointermove", move); window.addEventListener("pointerup", stop); }} />}
  </section>;
}
