import { useState } from "react";
import { ChevronDown, ChevronUp, X } from "lucide-react";
import { Button } from "@/components/ui/button";
import { CaptureViewer } from "./CaptureViewer";
import "./capture.css";

type CaptureWindowProps = {
  url?: string;
  downloadUrl?: string;
  title: string;
  onClose: () => void;
};

export function CaptureWindow({ url, downloadUrl, title, onClose }: CaptureWindowProps) {
  const [minimized, setMinimized] = useState(false);
  const [position, setPosition] = useState({ x: 24, y: 24, width: 960, height: 680 });
  const [drag, setDrag] = useState<{ x: number; y: number; pointerX: number; pointerY: number } | null>(null);

  return <section
    className={`capture-window ${minimized ? "minimized" : ""}`}
    style={{ left: position.x, top: position.y, width: minimized ? 240 : position.width, height: minimized ? 42 : position.height }}
  >
    <header
      onPointerDown={(event) => setDrag({ x: position.x, y: position.y, pointerX: event.clientX, pointerY: event.clientY })}
      onPointerMove={(event) => {
        if (!drag || !event.buttons) return;
        setPosition((current) => ({
          ...current,
          x: Math.max(0, Math.min(innerWidth - current.width, drag.x + event.clientX - drag.pointerX)),
          y: Math.max(0, Math.min(innerHeight - 42, drag.y + event.clientY - drag.pointerY)),
        }));
      }}
      onPointerUp={() => setDrag(null)}
    >
      {title}<span />
      <Button size="icon-xs" variant="ghost" aria-label={minimized ? "Restore capture viewer" : "Minimize capture viewer"} onClick={() => setMinimized((value) => !value)}>{minimized ? <ChevronUp /> : <ChevronDown />}</Button>
      <Button size="icon-xs" variant="ghost" aria-label="Close capture viewer" onClick={onClose}><X /></Button>
    </header>
    {!minimized && <CaptureViewer streamUrl={url} downloadUrl={downloadUrl} title={title} />}
    {!minimized && <div className="capture-window-resize" onPointerDown={(event) => {
      event.preventDefault();
      const start = { x: event.clientX, y: event.clientY, width: position.width, height: position.height };
      const move = (pointer: PointerEvent) => setPosition((current) => ({
        ...current,
        width: Math.max(800, Math.min(innerWidth - current.x, start.width + pointer.clientX - start.x)),
        height: Math.max(600, Math.min(innerHeight - current.y, start.height + pointer.clientY - start.y)),
      }));
      const stop = () => { window.removeEventListener("pointermove", move); window.removeEventListener("pointerup", stop); };
      window.addEventListener("pointermove", move);
      window.addEventListener("pointerup", stop);
    }} />}
  </section>;
}
