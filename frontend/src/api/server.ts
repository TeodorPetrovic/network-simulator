/**
 * Which controller the UI talks to. In a browser the UI is served by the controller itself, so every
 * request stays same-origin. The Tauri desktop app ships the UI locally and connects to a remote
 * controller chosen by the user; the runtime (QEMU, Dynamips, Docker, …) always runs on that server.
 */
const SERVER_KEY = "network-simulator.server";
const RECENT_KEY = "network-simulator.recent-servers";

export function isDesktop(): boolean {
  return typeof globalThis !== "undefined" && Boolean((globalThis as { isTauri?: unknown }).isTauri);
}

/** Accepts `host`, `host:3080`, or a full URL and returns a clean origin, or throws a readable error. */
export function normalizeServerUrl(input: string): string {
  const text = input.trim();
  if (!text) throw new Error("Enter the server address.");
  const withScheme = /^[a-z][a-z0-9+.-]*:\/\//i.test(text) ? text : `http://${text}`;
  let url: URL;
  try { url = new URL(withScheme); } catch { throw new Error("That is not a valid server address."); }
  if (url.protocol !== "http:" && url.protocol !== "https:") throw new Error("Use an http:// or https:// address.");
  if (!url.hostname) throw new Error("That is not a valid server address.");
  return url.origin;
}

function readStorage(key: string): string | null {
  try { return window.localStorage.getItem(key); } catch { return null; }
}
function writeStorage(key: string, value: string | null) {
  try { if (value === null) window.localStorage.removeItem(key); else window.localStorage.setItem(key, value); } catch { /* storage disabled */ }
}

/** The configured controller origin, or "" when the UI is same-origin (browser) or not configured yet. */
export function getServerOrigin(): string {
  if (!isDesktop()) return "";
  const stored = readStorage(SERVER_KEY);
  if (!stored) return "";
  try { return normalizeServerUrl(stored); } catch { return ""; }
}

export function setServerOrigin(input: string | null): string {
  if (input === null) { writeStorage(SERVER_KEY, null); return ""; }
  const origin = normalizeServerUrl(input);
  writeStorage(SERVER_KEY, origin);
  const recent = [origin, ...recentServers().filter((entry) => entry !== origin)].slice(0, 6);
  writeStorage(RECENT_KEY, JSON.stringify(recent));
  return origin;
}

export function recentServers(): string[] {
  try {
    const parsed = JSON.parse(readStorage(RECENT_KEY) ?? "[]") as unknown;
    return Array.isArray(parsed) ? parsed.filter((entry): entry is string => typeof entry === "string") : [];
  } catch { return []; }
}

/** Desktop app that has not been pointed at a server yet. */
export function needsServer(): boolean { return isDesktop() && !getServerOrigin(); }

/** Absolute base for building URLs: the controller origin (desktop) or this page (browser). */
export function serverBase(): string { return getServerOrigin() || window.location.href; }

/** Turn a controller path such as `/v3/projects` into a URL that reaches the right server. */
export function serverUrl(path: string): string {
  const origin = getServerOrigin();
  return origin && path.startsWith("/") ? `${origin}${path}` : path;
}

export function serverHostname(): string { return new URL(serverBase()).hostname; }

/** Sub-resource URLs that do not carry the page origin (iframe messages, ws checks). */
export function serverOriginOrSelf(): string { return getServerOrigin() || window.location.origin; }
