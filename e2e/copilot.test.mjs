// The AI copilot and MCP endpoint: configure a model, chat, and let the model call simulator tools.
import test from "node:test";
import assert from "node:assert/strict";
import http from "node:http";
import { startServer, newProject, launchPage, openProject } from "./harness.mjs";

function mockModel() {
  const requests = [];
  const server = http.createServer((req, res) => {
    let raw = "";
    req.on("data", (c) => (raw += c));
    req.on("end", () => {
      const body = JSON.parse(raw);
      requests.push(body);
      const last = body.messages.at(-1);
      const message = last.role === "tool"
        ? { role: "assistant", content: "Found a project named copilot-lab." }
        : { role: "assistant", content: "", tool_calls: [{ id: "c1", type: "function", function: { name: "project_list", arguments: "{}" } }] };
      res.writeHead(200, { "content-type": "application/json" });
      res.end(JSON.stringify({ choices: [{ message }] }));
    });
  });
  return new Promise((resolve) => server.listen(0, "127.0.0.1", () => resolve({ server, requests, url: `http://127.0.0.1:${server.address().port}` })));
}

test("copilot window configures a model and runs a tool-using chat", async (t) => {
  const model = await mockModel();
  const server = await startServer();
  const ui = await launchPage(server);
  const projectId = await newProject(server, "copilot-lab");
  t.after(async () => { await ui.close(); await server.stop(); model.server.close(); });
  await openProject(server, ui.page, projectId);
  const page = ui.page;
  await page.getByRole("menuitem", { name: /Tools/ }).click();
  await page.getByRole("menuitem", { name: "AI copilot…" }).click();
  const window = page.getByLabel("AI copilot");
  await window.waitFor();
  // First run opens the model settings.
  await window.getByLabel("Provider").selectOption("openai");
  await window.getByLabel("Model", { exact: true }).fill("mock-model");
  await window.getByLabel("Base URL").fill(model.url);
  await window.getByLabel("API key").fill("secret-key");
  await window.getByRole("button", { name: "Save model" }).click();
  await window.getByLabel("Message the copilot").fill("What projects do I have?");
  await window.getByRole("button", { name: "Send" }).click();
  await window.getByText("Found a project named copilot-lab.").waitFor();
  await window.getByText("project_list").waitFor();
  assert.ok(model.requests.length >= 2, "the model was called again with the tool result");
  // The stored key never comes back to the browser.
  const configs = (await server.api("GET", "/copilot/llm-configs")).json;
  assert.equal(configs[0].has_api_key, true);
  assert.equal(JSON.stringify(configs).includes("secret-key"), false);
  // The chat is kept.
  const sessions = (await server.api("GET", `/copilot/projects/${projectId}/chat/sessions`)).json;
  assert.equal(sessions.length, 1);
});

test("MCP endpoint lists tools and runs one", async (t) => {
  const server = await startServer();
  t.after(() => server.stop());
  const call = async (body) => (await server.api("POST", "/mcp", body)).json;
  const init = await call({ jsonrpc: "2.0", id: 1, method: "initialize", params: {} });
  assert.equal(init.result.serverInfo.name, "network-simulator");
  const tools = await call({ jsonrpc: "2.0", id: 2, method: "tools/list" });
  assert.ok(tools.result.tools.some((tool) => tool.name === "node_run_command"));
  const made = await call({ jsonrpc: "2.0", id: 3, method: "tools/call", params: { name: "project_create", arguments: { name: "via-mcp" } } });
  assert.equal(made.result.isError, false);
  const listed = await call({ jsonrpc: "2.0", id: 4, method: "tools/call", params: { name: "project_list", arguments: {} } });
  assert.match(listed.result.content[0].text, /via-mcp/);
  const bad = await call({ jsonrpc: "2.0", id: 5, method: "tools/call", params: { name: "project_get", arguments: {} } });
  assert.equal(bad.result.isError, true);
});

test("with auth enabled: MCP needs a token and tools run with the caller's permissions", async (t) => {
  const { spawn } = await import("node:child_process");
  const { mkdtempSync, readFileSync, rmSync } = await import("node:fs");
  const { tmpdir } = await import("node:os");
  const { join } = await import("node:path");
  const { freePortForTests } = await import("./harness.mjs");
  const dir = mkdtempSync(join(tmpdir(), "ns-mcp-"));
  const port = await freePortForTests();
  const base = `http://127.0.0.1:${port}`;
  const child = spawn("target/debug/network-simulator", [], { env: { ...process.env, NETWORK_SIMULATOR_BIND: `127.0.0.1:${port}`, NETWORK_SIMULATOR_DB: join(dir, "t.sqlite"), NETWORK_SIMULATOR_AUTH_ENABLED: "true" }, stdio: "ignore" });
  t.after(() => { child.kill("SIGKILL"); rmSync(dir, { recursive: true, force: true }); });
  for (let i = 0; i < 100; i++) { try { if ((await fetch(base + "/healthz")).ok) break; } catch {} await new Promise((r) => setTimeout(r, 100)); }
  const rpc = (token, name) => fetch(base + "/v3/mcp", { method: "POST", headers: { "content-type": "application/json", ...(token ? { authorization: `Bearer ${token}` } : {}) }, body: JSON.stringify({ jsonrpc: "2.0", id: 1, method: "tools/call", params: { name, arguments: {} } }) });
  assert.equal((await rpc(null, "project_list")).status, 401);
  const initial = readFileSync(join(dir, "initial-admin-password.txt"), "utf8").trim().split(/\s+/).pop();
  const form = (password) => ({ method: "POST", headers: { "content-type": "application/x-www-form-urlencoded" }, body: new URLSearchParams({ username: "admin", password }).toString() });
  let token = (await (await fetch(base + "/v3/access/users/login", form(initial))).json()).access_token;
  await fetch(base + "/v3/access/users/me", { method: "PUT", headers: { "content-type": "application/json", authorization: `Bearer ${token}` }, body: JSON.stringify({ password: "new-correct-password-1" }) });
  token = (await (await fetch(base + "/v3/access/users/login", form("new-correct-password-1"))).json()).access_token;
  const ok = await (await rpc(token, "project_list")).json();
  assert.equal(ok.result.isError, false, JSON.stringify(ok));
  const garbage = await (await rpc("not-a-token", "project_list")).status;
  assert.equal(garbage, 401);
});
