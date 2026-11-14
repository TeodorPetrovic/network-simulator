import { isDesktop, serverUrl } from "./server";

const AUTH_TOKEN_KEY = "network-simulator.access-token";
let authToken: string | null = null;
// Browsers keep the token per tab (sessionStorage). The desktop app keeps it across launches and
// shares it with its pop-out windows, which do not inherit a parent tab's sessionStorage.
const tokenStore = (): Storage => isDesktop() ? window.localStorage : window.sessionStorage;
if (!isDesktop()) try { window.localStorage.removeItem(AUTH_TOKEN_KEY); } catch { /* storage can be disabled by the browser */ }
export function getAuthToken(): string | null {
  try {
    const storedToken = tokenStore().getItem(AUTH_TOKEN_KEY);
    if (storedToken !== authToken) authToken = storedToken;
  } catch { /* retain the in-memory session if storage is disabled */ }
  return authToken;
}
export function saveAuthToken(token: string | null) {
  authToken = token;
  try {
    if (token) tokenStore().setItem(AUTH_TOKEN_KEY, token);
    else { tokenStore().removeItem(AUTH_TOKEN_KEY); saveRefreshToken(null); }
  } catch { /* storage can be disabled by the browser */ }
}

// The long-lived refresh token is kept across tabs and restarts so the short-lived access token can be
// renewed silently; signing out, changing the password or a rejected refresh clears it.
const REFRESH_TOKEN_KEY = "network-simulator.refresh-token";
export function saveRefreshToken(token: string | null) {
  try {
    if (token) window.localStorage.setItem(REFRESH_TOKEN_KEY, token);
    else window.localStorage.removeItem(REFRESH_TOKEN_KEY);
  } catch { /* storage can be disabled by the browser */ }
}
function readRefreshToken(): string | null {
  try { return window.localStorage.getItem(REFRESH_TOKEN_KEY); } catch { return null; }
}

let refreshing: Promise<boolean> | null = null;
/** Exchange the stored refresh token for a new access token. Refresh tokens are single use, so concurrent
 * callers share one request and other tabs wait on a lock instead of replaying the same token. */
export function refreshSession(staleToken: string | null): Promise<boolean> {
  refreshing ??= (async () => {
    const run = async (): Promise<boolean> => {
      // Another tab may have renewed the session while this one waited.
      const current = getAuthToken();
      if (current && current !== staleToken) return true;
      const refreshToken = readRefreshToken();
      if (!refreshToken) return false;
      try {
        const response = await fetch(serverUrl("/v3/access/users/refresh"), {
          method: "POST",
          headers: { "Content-Type": "application/json" },
          body: JSON.stringify({ refresh_token: refreshToken }),
        });
        if (!response.ok) { if (response.status === 401) saveAuthToken(null); return false; }
        const body = await response.json() as { access_token?: string; refresh_token?: string };
        if (!body.access_token) return false;
        saveAuthToken(body.access_token);
        if (body.refresh_token) saveRefreshToken(body.refresh_token);
        return true;
      } catch { return false; }
    };
    try {
      const locks = typeof navigator !== "undefined" ? navigator.locks : undefined;
      return locks ? await locks.request("network-simulator.refresh", run) : await run();
    } finally { refreshing = null; }
  })();
  return refreshing;
}

function sendWithToken(input: RequestInfo | URL, init: RequestInit | undefined, token: string | null): Promise<Response> {
  if (!token) return init ? fetch(input, init) : fetch(input);
  return fetch(input, {
    ...init,
    headers: { ...(init?.headers as Record<string, string> | undefined), Authorization: `Bearer ${token}` },
  });
}
export async function fetchWithAuth(input: RequestInfo | URL, init?: RequestInit): Promise<Response> {
  const token = getAuthToken();
  if (typeof input === "string") input = serverUrl(input);
  const response = await sendWithToken(input, init, token);
  const url = typeof input === "string" ? input : input instanceof URL ? input.href : input.url;
  // Expired or missing access token: renew it once with the refresh token and replay the request.
  if (response.status !== 401 || /\/access\/users\/(login|refresh|authenticate)/.test(url) || !readRefreshToken()) return response;
  if (!(await refreshSession(token))) return response;
  return sendWithToken(input, init, getAuthToken());
}
export function handleUnauthorized(response: Response, path: string) {
  if (response.status !== 401 || path.startsWith("/access/users/login") || path.startsWith("/access/users/authenticate")) return;
  saveAuthToken(null);
  if (typeof window !== "undefined" && window.location.pathname !== "/login") {
    const returnTo = `${window.location.pathname}${window.location.search}`;
    window.location.assign(`/login?returnTo=${encodeURIComponent(returnTo)}`);
  }
}

export async function request<T>(path: string, init: RequestInit = {}): Promise<T> {
  const response = await fetchWithAuth(`/v3${path}`, {
    ...init,
    headers: {
      ...(init.body ? { "Content-Type": "application/json" } : {}),
      ...init.headers,
    },
  });
  handleUnauthorized(response, path);

  if (!response.ok) {
    const body = (await response.json().catch(() => null)) as { message?: string } | null;
    if (response.status === 401 && !path.startsWith("/access/users/login") && !path.startsWith("/access/users/authenticate")) {
      saveAuthToken(null);
      if (typeof window !== "undefined" && window.location.pathname !== "/login") {
        const returnTo = `${window.location.pathname}${window.location.search}`;
        window.location.assign(`/login?returnTo=${encodeURIComponent(returnTo)}`);
      }
    }
    throw new Error(body?.message ?? `Request failed (${response.status})`);
  }
  if (response.status === 204) return undefined as T;
  return (await response.json()) as T;
}

export const jsonBody = (value: unknown): Pick<RequestInit, "body" | "method"> => ({
  method: "POST",
  body: JSON.stringify(value),
});

