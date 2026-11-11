import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { fetchWithAuth, saveAuthToken } from "./client";
import { getServerOrigin, isDesktop, needsServer, normalizeServerUrl, recentServers, serverBase, serverHostname, serverUrl, setServerOrigin } from "./server";
import { probeServer } from "../routes/connect";
import { webWiresharkUrl, projectNotificationsUrl } from "./index";

const setDesktop = (on: boolean) => { (globalThis as { isTauri?: boolean }).isTauri = on || undefined; };

beforeEach(() => { window.localStorage.clear(); saveAuthToken(null); });
afterEach(() => { setDesktop(false); vi.unstubAllGlobals(); });

describe("normalizeServerUrl", () => {
  it("adds http and strips paths", () => {
    expect(normalizeServerUrl("lab:3080")).toBe("http://lab:3080");
    expect(normalizeServerUrl(" https://lab.example.com/app/ ")).toBe("https://lab.example.com");
  });
  it("rejects empty and non-http input", () => {
    expect(() => normalizeServerUrl("")).toThrow();
    expect(() => normalizeServerUrl("ftp://lab")).toThrow(/http/);
  });
});

describe("browser (same-origin) mode", () => {
  it("never rewrites paths and ignores any stored server", () => {
    window.localStorage.setItem("network-simulator.server", "http://elsewhere:1");
    expect(isDesktop()).toBe(false);
    expect(getServerOrigin()).toBe("");
    expect(needsServer()).toBe(false);
    expect(serverUrl("/v3/projects")).toBe("/v3/projects");
  });
});

describe("desktop mode", () => {
  beforeEach(() => setDesktop(true));

  it("needs a server until one is saved, and remembers recent ones", () => {
    expect(needsServer()).toBe(true);
    setServerOrigin("lab:3080");
    setServerOrigin("https://other.example");
    expect(needsServer()).toBe(false);
    expect(getServerOrigin()).toBe("https://other.example");
    expect(recentServers()).toEqual(["https://other.example", "http://lab:3080"]);
    expect(serverHostname()).toBe("other.example");
  });

  it("routes API paths, websocket URLs and bearer tokens to the server", async () => {
    setServerOrigin("http://lab:3080");
    expect(serverUrl("/v3/projects")).toBe("http://lab:3080/v3/projects");
    expect(serverUrl("https://x.example/a")).toBe("https://x.example/a");
    expect(serverBase()).toBe("http://lab:3080");
    expect(webWiresharkUrl("/v3/capture/ws")).toBe("ws://lab:3080/v3/capture/ws");
    const fetchMock = vi.fn(async () => new Response("{}"));
    vi.stubGlobal("fetch", fetchMock);
    saveAuthToken("tok");
    await fetchWithAuth("/v3/projects");
    const [url, init] = fetchMock.mock.calls[0] as unknown as [string, RequestInit];
    expect(url).toBe("http://lab:3080/v3/projects");
    expect((init.headers as Record<string, string>).Authorization).toBe("Bearer tok");
  });

  it("uses wss for an https server when minting console tickets", async () => {
    setServerOrigin("https://lab.example.com");
    expect(await projectNotificationsUrl("p1", 4)).toBe("wss://lab.example.com/v3/projects/p1/notifications/ws?after=4");
  });
});

describe("probeServer", () => {
  it("returns the origin when /healthz answers", async () => {
    const fetcher = vi.fn(async () => new Response("ok")) as unknown as typeof fetch;
    expect(await probeServer("lab", fetcher)).toBe("http://lab");
    expect((fetcher as unknown as ReturnType<typeof vi.fn>).mock.calls[0][0]).toBe("http://lab/healthz");
  });
  it("explains unreachable servers and non-controller answers", async () => {
    await expect(probeServer("lab", (async () => { throw new TypeError("Failed to fetch"); }) as typeof fetch)).rejects.toThrow(/Could not reach http:\/\/lab/);
    await expect(probeServer("lab", (async () => new Response("no", { status: 404 })) as typeof fetch)).rejects.toThrow(/answered 404/);
  });
});
