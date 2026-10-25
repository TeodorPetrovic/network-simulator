import { useEffect, useMemo, useRef, useState } from "react";
import { ChevronDown, ChevronUp, LoaderCircle, RotateCw, X } from "lucide-react";
import { Button } from "@/components/ui/button";
import { api, webWiresharkClientUrl } from "../api";
import "./capture.css";

type Props = { projectId: string; linkId: string; wsPath: string; title: string; onClose: () => void };

export function WebWiresharkWindow({ projectId, linkId, wsPath, title, onClose }: Props) {
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
      if (event.source !== frameRef.current?.contentWindow || event.origin !== window.location.origin || event.data?.type !== "xpra-connected") return;
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
  return <section className={`capture-window ${minimized ? "minimized" : ""}`} style={{ left: position.x, top: position.y, width: minimized ? 340 : position.width, height: minimized ? 42 : position.height }} aria-label={title}>
    <header onPointerDown={(event) => {
      if ((event.target as HTMLElement).closest("button")) return;
      const x = event.clientX, y = event.clientY, startX = position.x, startY = position.y;
      const move = (pointer: PointerEvent) => setPosition((current) => ({ ...current, x: Math.max(0, Math.min(innerWidth - current.width, startX + pointer.clientX - x)), y: Math.max(0, Math.min(innerHeight - 42, startY + pointer.clientY - y)) }));
      const stop = () => { window.removeEventListener("pointermove", move); window.removeEventListener("pointerup", stop); };
      window.addEventListener("pointermove", move); window.addEventListener("pointerup", stop);
    }}>{title}<span />{!minimized && <Button size="sm" variant="ghost" onClick={() => void restart()}><RotateCw />Restart Wireshark</Button>}<Button size="icon-xs" variant="ghost" aria-label={minimized ? "Restore Web Wireshark" : "Minimize Web Wireshark"} onClick={() => setMinimized((value) => !value)}>{minimized ? <ChevronUp /> : <ChevronDown />}</Button><Button size="icon-xs" variant="ghost" aria-label="Close Web Wireshark" onClick={onClose}><X /></Button></header>
    {!minimized && <div className="web-wireshark-frame">{!loaded && <div className="web-wireshark-loading" role="status"><LoaderCircle className="animate-spin" />{error || "Connecting to Web Wireshark…"}{error && <Button size="sm" onClick={() => { setError(""); setLoaded(false); setFrameKey((value) => value + 1); }}>Retry</Button>}</div>}<iframe ref={frameRef} key={frameKey} title="Web Wireshark" src={source} />{!error && !loaded && <span className="sr-only">Waiting for the Wireshark session to connect</span>}</div>}
    {!minimized && <div className="capture-window-resize" onPointerDown={(event) => { event.preventDefault(); const x = event.clientX, y = event.clientY, width = position.width, height = position.height; const move = (pointer: PointerEvent) => setPosition((current) => ({ ...current, width: Math.max(800, Math.min(1920, innerWidth - current.x, width + pointer.clientX - x)), height: Math.max(600, Math.min(1080, innerHeight - current.y, height + pointer.clientY - y)) })); const stop = () => { window.removeEventListener("pointermove", move); window.removeEventListener("pointerup", stop); }; window.addEventListener("pointermove", move); window.addEventListener("pointerup", stop); }} />}
  </section>;
}
