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
    else tokenStore().removeItem(AUTH_TOKEN_KEY);
  } catch { /* storage can be disabled by the browser */ }
}
export function fetchWithAuth(input: RequestInfo | URL, init?: RequestInit): Promise<Response> {
  const token = getAuthToken();
  if (typeof input === "string") input = serverUrl(input);
  if (!token) return init ? fetch(input, init) : fetch(input);
  return fetch(input, {
    ...init,
    headers: { ...(init?.headers as Record<string, string> | undefined), Authorization: `Bearer ${token}` },
  });
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

