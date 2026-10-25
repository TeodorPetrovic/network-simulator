// Shared helpers for the browser end-to-end suite (node:test + playwright-core).
// Run everything with `scripts/e2e.sh` (builds nothing; expects target/debug/network-simulator
// and frontend/dist/client to exist: `cargo build` and `cd frontend && npm run build`).
import { spawn } from "node:child_process";
import { mkdtempSync, rmSync, existsSync } from "node:fs";
import { tmpdir } from "node:os";
import { join, resolve, dirname } from "node:path";
import { fileURLToPath } from "node:url";
import net from "node:net";
import { createRequire } from "node:module";

const root = resolve(dirname(fileURLToPath(import.meta.url)), "..");
const pwPath = process.env.PLAYWRIGHT_CORE ?? join(process.env.HOME, ".cache/qa-tools/node_modules/playwright-core");
const playwright = createRequire(import.meta.url)(pwPath);

export const browsers = { chromium: playwright.chromium, firefox: playwright.firefox };
export const browserName = process.env.E2E_BROWSER ?? "chromium";

function freePort() {
  return new Promise((ok, fail) => {
    const s = net.createServer();
    s.listen(0, "127.0.0.1", () => { const { port } = s.address(); s.close(() => ok(port)); });
    s.on("error", fail);
  });
}

/** Start an isolated simulator (loopback => auth off, temp DB) and return {base, api, stop}. */
export async function startServer() {
  const bin = process.env.E2E_BINARY ?? join(root, "target/debug/network-simulator");
  if (!existsSync(bin)) throw new Error(`missing ${bin}; run cargo build first`);
  const dir = mkdtempSync(join(tmpdir(), "ns-e2e-"));
  const port = await freePort();
  const child = spawn(bin, [], {
    cwd: root,
    env: { ...process.env, NETWORK_SIMULATOR_BIND: `127.0.0.1:${port}`, NETWORK_SIMULATOR_DB: join(dir, "t.sqlite") },
    stdio: ["ignore", "pipe", "pipe"],
  });
  let log = "";
  child.stdout.on("data", (d) => { log += d; }); child.stderr.on("data", (d) => { log += d; });
  const base = `http://127.0.0.1:${port}`;
  for (let i = 0; i < 100; i++) {
    try { if ((await fetch(base + "/healthz")).ok) break; } catch {}
    await new Promise((r) => setTimeout(r, 100));
    if (i === 99) { child.kill(); throw new Error("server did not start:\n" + log.slice(-2000)); }
  }
  const api = async (method, path, body) => {
    const res = await fetch(base + "/v3" + path, { method, headers: { "content-type": "application/json" }, body: body === undefined ? undefined : JSON.stringify(body) });
    const text = await res.text();
    let json; try { json = JSON.parse(text); } catch { json = text; }
    return { status: res.status, json };
  };
  return {
    base, api, log: () => log,
    async stop() { child.kill("SIGTERM"); await new Promise((r) => setTimeout(r, 300)); child.kill("SIGKILL"); rmSync(dir, { recursive: true, force: true }); },
  };
}

export async function newProject(server, name = "e2e") {
  const r = await server.api("POST", "/projects", { name });
  if (r.status !== 201) throw new Error("create project failed " + JSON.stringify(r));
  return r.json.project_id;
}

export async function templateByType(server, type, nameIncludes) {
  const { json } = await server.api("GET", "/templates");
  const t = json.find((x) => x.template_type === type && (!nameIncludes || x.name.includes(nameIncludes)));
  if (!t) throw new Error(`no template of type ${type}`);
  return t;
}

/** Add a node from a template through the API at scene coords. */
export async function addNode(server, projectId, template, x = 0, y = 0, name) {
  const r = await server.api("POST", `/projects/${projectId}/templates/${template.template_id}`, { x, y, compute_id: "local", ...(name ? { name } : {}) });
  if (r.status !== 201) throw new Error("add node failed " + JSON.stringify(r));
  return r.json;
}

export async function launchPage(server, { width = 1500, height = 900 } = {}) {
  const browser = await browsers[browserName].launch();
  const page = await browser.newPage({ viewport: { width, height } });
  page.setDefaultTimeout(8000);
  const errors = [];
  page.on("pageerror", (e) => errors.push("pageerror: " + e.message));
  page.on("console", (m) => {
    if (m.type() !== "error" || /Failed to load resource/.test(m.text())) return;
    // Firefox reports a transient WebSocket connection error when a page reloads
    // while its project event stream is reconnecting. The server still serves
    // the page and the next stream connection is established normally.
    if (browserName === "firefox" && /notifications\/ws/.test(m.text()) && /can’t establish a connection|connection .* interrupted while the page was loading/i.test(m.text())) return;
    errors.push("console: " + m.text());
  });
  return { browser, page, errors, close: () => browser.close() };
}

export async function openProject(server, page, projectId) {
  await page.goto(`${server.base}/projects/${projectId}`);
  await page.locator(".sim-workspace").waitFor();
}

/** Poll until fn() returns truthy. */
export async function waitFor(fn, { timeout = 15000, interval = 250, label = "condition" } = {}) {
  const end = Date.now() + timeout;
  for (;;) {
    const v = await fn();
    if (v) return v;
    if (Date.now() > end) throw new Error("timeout waiting for " + label);
    await new Promise((r) => setTimeout(r, interval));
  }
}
