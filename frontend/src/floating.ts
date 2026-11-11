import { openAppWindow } from "./api/desktop";
import { isDesktop } from "./api/server";
import { useCallback, useEffect, useReducer, useRef } from "react";

// Floating windows (consoles, capture viewers, Web Wireshark) share one ordered stack so the
// window the user touched last is drawn on top. Stack positions map to z-index 30-49, which
// keeps them below dialogs, menus and popovers (z-index 50).
const stack: object[] = [];
const listeners = new Set<() => void>();
const notify = () => listeners.forEach((listener) => listener());
const BASE = 30;
const MAX_STEPS = 19;

export function useFloatingFront() {
  const id = useRef<object>({}).current;
  const [, rerender] = useReducer((count: number) => count + 1, 0);
  useEffect(() => {
    if (!stack.includes(id)) stack.push(id);
    listeners.add(rerender);
    notify();
    return () => {
      const index = stack.indexOf(id);
      if (index >= 0) stack.splice(index, 1);
      listeners.delete(rerender);
      notify();
    };
  }, [id]);
  const front = useCallback(() => {
    const index = stack.indexOf(id);
    if (index < 0 || index === stack.length - 1) return;
    stack.splice(index, 1);
    stack.push(id);
    notify();
  }, [id]);
  const index = stack.indexOf(id);
  const z = BASE + Math.min(index < 0 ? stack.length : index, MAX_STEPS);
  return { z, front };
}

// Floating windows may be dragged mostly off-screen (like minimised-to-edge windows); this much of the
// title bar always stays reachable so the window can be dragged back.
const GRAB = 120;
export function clampFloat(x: number, y: number, width: number) {
  return { x: Math.max(GRAB - width, Math.min(window.innerWidth - GRAB, x)), y: Math.max(0, Math.min(window.innerHeight - 42, y)) };
}

export function popOutWindow(url: string, width = 1000, height = 700, title = "Network Studio") {
  if (isDesktop()) {
    // A desktop webview has no window.open: ask the shell for a native window. External URLs
    // (Web Wireshark's Xpra client) are wrapped in the in-app /embed page.
    const route = /^https?:\/\//i.test(url) ? `/embed?${new URLSearchParams({ url, title })}` : url;
    void openAppWindow(route, title, width, height).catch(() => undefined);
    return null;
  }
  const left = Math.max(0, Math.round(window.screenX + (window.outerWidth - width) / 2));
  const top = Math.max(0, Math.round(window.screenY + (window.outerHeight - height) / 2));
  // No "noopener": a same-origin popup inherits sessionStorage, which carries the access token.
  return window.open(url, "_blank", `popup=yes,width=${width},height=${height},left=${left},top=${top}`);
}
