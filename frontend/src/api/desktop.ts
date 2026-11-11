/** Bridge to the Tauri shell (desktop app only) plus the user's console/VNC viewer choice. */
import { isDesktop } from "./server";

export type ToolId = "wireshark" | "motty";
export type ToolStatus = { id: ToolId; name: string; installed: boolean; path: string | null; can_install: boolean; note: string };
export type ViewerChoice = "builtin" | "motty";
export type DesktopPrefs = { console: ViewerChoice; vnc: ViewerChoice; capture: "builtin" | "wireshark" };

const PREFS_KEY = "network-studio.desktop-prefs";
const DEFAULT_PREFS: DesktopPrefs = { console: "builtin", vnc: "builtin", capture: "builtin" };

type Invoke = (command: string, args?: Record<string, unknown>) => Promise<unknown>;
function invoker(): Invoke {
  const internals = (globalThis as { __TAURI_INTERNALS__?: { invoke?: Invoke } }).__TAURI_INTERNALS__;
  if (!internals?.invoke) throw new Error("This action is only available in the desktop app.");
  return internals.invoke;
}
export function desktopInvoke<T>(command: string, args?: Record<string, unknown>): Promise<T> {
  try { return invoker()(command, args) as Promise<T>; } catch (error) { return Promise.reject(error); }
}

export function getDesktopPrefs(): DesktopPrefs {
  try {
    const raw = JSON.parse(window.localStorage.getItem(PREFS_KEY) ?? "{}") as Partial<DesktopPrefs>;
    return {
      console: raw.console === "motty" ? "motty" : "builtin",
      vnc: raw.vnc === "motty" ? "motty" : "builtin",
      capture: raw.capture === "wireshark" ? "wireshark" : "builtin",
    };
  } catch { return { ...DEFAULT_PREFS }; }
}
export function saveDesktopPrefs(prefs: DesktopPrefs) {
  try { window.localStorage.setItem(PREFS_KEY, JSON.stringify(prefs)); } catch { /* storage disabled */ }
}

export const listTools = () => desktopInvoke<ToolStatus[]>("tool_status");
export const installTool = (id: ToolId) => desktopInvoke<ToolStatus[]>("install_tool", { id });
export const launchMotty = (target?: string) => desktopInvoke<void>("launch_motty", { target: target ?? null });
export const launchWireshark = (streamUrl: string, title: string) => desktopInvoke<void>("launch_wireshark", { streamUrl, title });
export const openInWireshark = (fileUrl: string, name: string) => desktopInvoke<void>("open_in_wireshark", { fileUrl, name });

/** Opens an in-app route (`/console?...`) in a real native window. */
export function openAppWindow(url: string, title: string, width = 1000, height = 700): Promise<string> {
  return desktopInvoke<string>("open_window", { url, title, width, height });
}

/** Subscribes to install progress events using the raw Tauri event protocol (no extra dependency). */
export async function listenToolProgress(handler: (event: { id: string; stage: string; done: number; total: number }) => void): Promise<() => void> {
  const internals = (globalThis as { __TAURI_INTERNALS__?: { invoke?: Invoke; transformCallback?: (cb: (e: { id: number; payload: never }) => void) => number } }).__TAURI_INTERNALS__;
  if (!isDesktop() || !internals?.invoke || !internals.transformCallback) return () => {};
  const callback = internals.transformCallback((event) => handler(event.payload as never));
  const eventId = await internals.invoke("plugin:event|listen", { event: "tool-progress", target: { kind: "Any" }, handler: callback }) as number;
  return () => { void internals.invoke!("plugin:event|unlisten", { event: "tool-progress", eventId }); };
}

/** Opens Motty on a link (`telnet://host:port/`). */
export async function openInMottyLink(link: string): Promise<string> {
  await launchMotty(link);
  return `Opened ${link} in Motty.`;
}

/** Opens a VNC session in Motty with `motty vnc <host> <port>`. */
export async function openVncInMotty(host: string, port: number): Promise<string> {
  await desktopInvoke<void>("launch_motty_vnc", { host, port });
  return `Opened ${host}:${port} in Motty.`;
}
