import { useEffect, useRef, useState } from "react";

type Tip = { text: string; x: number; y: number; below: boolean };

/**
 * Replaces the browser's native `title` tooltips (unstyled, slow, OS-themed) with one consistent
 * tooltip. The `title` attribute stays in the markup (and is restored on leave), so nothing else changes.
 */
export function TitleTooltips() {
  const [tip, setTip] = useState<Tip | null>(null);
  const active = useRef<{ el: HTMLElement; title: string } | null>(null);
  const timer = useRef<number>(0);

  useEffect(() => {
    const restore = () => {
      window.clearTimeout(timer.current);
      const current = active.current;
      if (current) { if (current.el.isConnected && !current.el.hasAttribute("title")) current.el.setAttribute("title", current.title); active.current = null; }
      setTip(null);
    };
    const over = (event: PointerEvent) => {
      if (event.pointerType === "touch") return;
      const el = (event.target as Element | null)?.closest?.("[title]") as HTMLElement | null;
      if (!el || el === active.current?.el) return;
      restore();
      const title = el.getAttribute("title")?.trim();
      if (!title) return;
      el.removeAttribute("title");
      if (!el.hasAttribute("aria-label") && !el.textContent?.trim()) el.setAttribute("aria-label", title);
      active.current = { el, title };
      timer.current = window.setTimeout(() => {
        const rect = el.getBoundingClientRect();
        const below = rect.top < 48;
        setTip({ text: title, x: rect.left + rect.width / 2, y: below ? rect.bottom : rect.top, below });
      }, 350);
    };
    const out = (event: PointerEvent) => {
      const current = active.current;
      if (current && !current.el.contains(event.relatedTarget as Node | null)) restore();
    };
    document.addEventListener("pointerover", over);
    document.addEventListener("pointerout", out);
    document.addEventListener("pointerdown", restore, true);
    document.addEventListener("keydown", restore, true);
    window.addEventListener("scroll", restore, true);
    return () => {
      restore();
      document.removeEventListener("pointerover", over);
      document.removeEventListener("pointerout", out);
      document.removeEventListener("pointerdown", restore, true);
      document.removeEventListener("keydown", restore, true);
      window.removeEventListener("scroll", restore, true);
    };
  }, []);

  if (!tip) return null;
  const half = 140;
  const x = Math.max(half + 8, Math.min(window.innerWidth - half - 8, tip.x));
  return <div role="tooltip" className="app-tooltip app-title-tip" style={{ left: x, top: tip.y, transform: `translate(-50%, ${tip.below ? "8px" : "calc(-100% - 8px)"})` }}>{tip.text}</div>;
}
