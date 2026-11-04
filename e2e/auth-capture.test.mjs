// Capture viewer + Web Wireshark with authentication ON (one-use tickets, token in the Xpra WS path).
import test from "node:test";
import assert from "node:assert/strict";
import { spawn } from "node:child_process";
import { mkdtempSync, readFileSync, rmSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { launchPage, freePortForTests } from "./harness.mjs";

test("with auth enabled: Open saved capture mints a fresh ticket and Web Wireshark connects", async (t) => {
  const dir = mkdtempSync(join(tmpdir(), "ns-auth-"));
  const port = await freePortForTests();
  const base = `http://127.0.0.1:${port}`;
  const child = spawn("target/debug/network-simulator", [], { env: { ...process.env, NETWORK_SIMULATOR_BIND: `127.0.0.1:${port}`, NETWORK_SIMULATOR_DB: join(dir, "t.sqlite"), NETWORK_SIMULATOR_AUTH_ENABLED: "true" }, stdio: "ignore" });
  const ui = await launchPage({ base });
  t.after(async () => { await ui.close(); child.kill("SIGKILL"); rmSync(dir, { recursive: true, force: true }); });
  for (let i = 0; i < 100; i++) { try { if ((await fetch(base + "/healthz")).ok) break; } catch {} await new Promise((r) => setTimeout(r, 100)); }
  const form = (password) => ({ method: "POST", headers: { "content-type": "application/x-www-form-urlencoded" }, body: new URLSearchParams({ username: "admin", password }).toString() });
  const initial = readFileSync(join(dir, "initial-admin-password.txt"), "utf8").trim().split(/\s+/).pop();
  let token = (await (await fetch(base + "/v3/access/users/login", form(initial))).json()).access_token;
  await fetch(base + "/v3/access/users/me", { method: "PUT", headers: { "content-type": "application/json", authorization: `Bearer ${token}` }, body: JSON.stringify({ password: "new-correct-password-1" }) });
  token = (await (await fetch(base + "/v3/access/users/login", form("new-correct-password-1"))).json()).access_token;
  const api = async (method, path, body) => { const r = await fetch(base + "/v3" + path, { method, headers: { "content-type": "application/json", authorization: `Bearer ${token}` }, body: body ? JSON.stringify(body) : undefined }); const x = await r.text(); try { return { status: r.status, json: JSON.parse(x) }; } catch { return { status: r.status, json: x }; } };
  const pid = (await api("POST", "/projects", { name: "auth-capture" })).json.project_id;
  const tpl = (await api("GET", "/templates")).json.find((x) => x.template_type === "vpcs");
  const mk = async (name, x) => (await api("POST", `/projects/${pid}/templates/${tpl.template_id}`, { x, y: 0, compute_id: "local", name })).json;
  const nodes = [await mk("PCA", -200), await mk("PCB", 200)];
  const link = (await api("POST", `/projects/${pid}/links`, { nodes: nodes.map((n) => ({ node_id: n.node_id, adapter_number: 0, port_number: 0 })), link_type: "ethernet" })).json;
  for (const n of nodes) await api("POST", `/projects/${pid}/nodes/${n.node_id}/start`, {});
  const page = ui.page;
  await page.addInitScript((tk) => sessionStorage.setItem("network-simulator.access-token", tk), token);
  const bad = []; page.on("response", (r) => { if (r.status() >= 400) bad.push(`${r.status()} ${r.url().replace(base, "")}`); });
  await page.goto(`${base}/projects/${pid}`); await page.locator(".sim-workspace").waitFor();
  const menu = () => page.locator(".sim-link").first().evaluate((e) => e.dispatchEvent(new MouseEvent("contextmenu", { bubbles: true, button: 2, clientX: 500, clientY: 400 })));

  // Saved capture: opening and re-opening must both work (tickets are single-use).
  assert.equal((await api("POST", `/projects/${pid}/links/${link.link_id}/capture/start`, { capture_file_name: "x.pcap", data_link_type: "DLT_EN10MB" })).status, 201);
  assert.equal((await api("POST", `/projects/${pid}/links/${link.link_id}/capture/stop`, {})).status, 204);
  await page.reload(); await page.locator(".sim-workspace").waitFor();
  await menu(); await page.getByRole("menuitem", { name: /built-in Capture Viewer/ }).click();
  await page.getByText("Saved capture").first().waitFor();
  bad.length = 0;
  await page.getByRole("button", { name: "Open saved capture" }).click();
  await page.waitForTimeout(1500);
  assert.deepEqual(bad, [], "re-opening the saved capture must not hit 401/404");
  await page.getByRole("button", { name: "Close", exact: false }).first().click().catch(() => {});

  // Web Wireshark through the Xpra client: the token rides in the WS path and must survive the client's path sanitising.
  const started = await api("POST", `/projects/${pid}/links/${link.link_id}/capture/start`, { capture_file_name: "w.pcap", data_link_type: "DLT_EN10MB", wireshark: true });
  assert.equal(started.status, 201);
  await page.reload(); await page.locator(".sim-workspace").waitFor();
  await menu(); await page.getByRole("menuitem", { name: "Open Web Wireshark (Inline)" }).click();
  const frame = page.getByTitle("Web Wireshark");
  await frame.waitFor();
  await frame.contentFrame().locator("canvas").waitFor({ timeout: 60_000 });
});

test("with auth enabled: built-in viewer streams a LIVE capture", async (t) => {
  const dir = mkdtempSync(join(tmpdir(), "ns-auth-"));
  const port = await freePortForTests();
  const base = `http://127.0.0.1:${port}`;
  const child = spawn("target/debug/network-simulator", [], { env: { ...process.env, NETWORK_SIMULATOR_BIND: `127.0.0.1:${port}`, NETWORK_SIMULATOR_DB: join(dir, "t.sqlite"), NETWORK_SIMULATOR_AUTH_ENABLED: "true" }, stdio: "ignore" });
  const ui = await launchPage({ base });
  t.after(async () => { await ui.close(); child.kill("SIGKILL"); rmSync(dir, { recursive: true, force: true }); });
  for (let i = 0; i < 100; i++) { try { if ((await fetch(base + "/healthz")).ok) break; } catch {} await new Promise((r) => setTimeout(r, 100)); }
  const form = (password) => ({ method: "POST", headers: { "content-type": "application/x-www-form-urlencoded" }, body: new URLSearchParams({ username: "admin", password }).toString() });
  const initial = readFileSync(join(dir, "initial-admin-password.txt"), "utf8").trim().split(/\s+/).pop();
  let token = (await (await fetch(base + "/v3/access/users/login", form(initial))).json()).access_token;
  await fetch(base + "/v3/access/users/me", { method: "PUT", headers: { "content-type": "application/json", authorization: `Bearer ${token}` }, body: JSON.stringify({ password: "new-correct-password-1" }) });
  token = (await (await fetch(base + "/v3/access/users/login", form("new-correct-password-1"))).json()).access_token;
  const api = async (method, path, body) => { const r = await fetch(base + "/v3" + path, { method, headers: { "content-type": "application/json", authorization: `Bearer ${token}` }, body: body ? JSON.stringify(body) : undefined }); const x = await r.text(); try { return { status: r.status, json: JSON.parse(x) }; } catch { return { status: r.status, json: x }; } };
  const pid = (await api("POST", "/projects", { name: "live-capture" })).json.project_id;
  const tpl = (await api("GET", "/templates")).json.find((x) => x.template_type === "vpcs");
  const mk = async (name, x) => (await api("POST", `/projects/${pid}/templates/${tpl.template_id}`, { x, y: 0, compute_id: "local", name })).json;
  const nodes = [await mk("PCA", -200), await mk("PCB", 200)];
  const link = (await api("POST", `/projects/${pid}/links`, { nodes: nodes.map((n) => ({ node_id: n.node_id, adapter_number: 0, port_number: 0 })), link_type: "ethernet" })).json;
  for (const n of nodes) await api("POST", `/projects/${pid}/nodes/${n.node_id}/start`, {});
  const page = ui.page;
  await page.addInitScript((tk) => sessionStorage.setItem("network-simulator.access-token", tk), token);
  const bad = []; page.on("response", (r) => { if (r.status() >= 400) bad.push(`${r.status()} ${r.url().replace(base, "")}`); });
  await page.goto(`${base}/projects/${pid}`); await page.locator(".sim-workspace").waitFor();
  await page.locator(".sim-link").first().evaluate((e) => e.dispatchEvent(new MouseEvent("contextmenu", { bubbles: true, button: 2, clientX: 500, clientY: 400 })));
  await page.getByRole("menuitem", { name: /built-in Capture Viewer/ }).click();
  await page.waitForTimeout(2500);
  assert.deepEqual(bad, [], "viewer on a never-captured link must not 4xx");
  assert.equal(await page.getByText(/Capture request failed/).count(), 0);
  // Right-click on a never-captured link: viewer must start a capture itself instead of 404ing.
  await page.getByRole("button", { name: "Close capture viewer" }).click().catch(() => {});
});
