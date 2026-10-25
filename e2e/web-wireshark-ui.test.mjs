import test from "node:test";
import assert from "node:assert/strict";
import { execFileSync } from "node:child_process";
import { startServer, newProject, launchPage, openProject, templateByType, addNode, waitFor } from "./harness.mjs";

const image = "network-simulator-web-wireshark:test";
const docker = (...args) => execFileSync("docker", args, { encoding: "utf8" }).trim();

test("Web Wireshark starts from the link dialog, opens inline, and closes when capture stops", async (t) => {
  try { docker("image", "inspect", image); } catch { return t.skip(`Docker image ${image} is unavailable`); }
  const server = await startServer();
  const ui = await launchPage(server);
  const pid = await newProject(server, "web-wireshark-ui");
  t.after(async () => { await ui.close(); await server.stop(); });
  const setting = await server.api("PUT", "/settings", { WebWireshark: { enabled: true, image } });
  assert.equal(setting.status, 200);
  const a = await addNode(server, pid, await templateByType(server, "vpcs"), 0, 0, "PC A");
  const b = await addNode(server, pid, await templateByType(server, "vpcs"), 220, 0, "PC B");
  const connected = await server.api("POST", `/projects/${pid}/links`, { nodes: [{ node_id: a.node_id, adapter_number: 0, port_number: 0 }, { node_id: b.node_id, adapter_number: 0, port_number: 0 }], link_type: "ethernet" });
  assert.equal(connected.status, 201);
  await Promise.all([a, b].map((node) => server.api("POST", `/projects/${pid}/nodes/${node.node_id}/start`, {})));
  await openProject(server, ui.page, pid);
  const page = ui.page;
  const line = page.locator(".sim-link").first();
  await line.click({ button: "right" });
  await page.getByRole("menuitem", { name: "Packet capture…" }).click();
  const dialog = page.getByRole("dialog", { name: "Link properties" });
  await dialog.getByRole("checkbox", { name: "Web Wireshark" }).check();
  await dialog.getByRole("button", { name: "Start capture" }).click();
  await page.getByText("Starting Web Wireshark… (3-6 s)").waitFor();
  await waitFor(async () => (await server.api("GET", `/projects/${pid}/links`)).json.find((item) => item.link_id === connected.json.link_id)?.wireshark === true, { timeout: 90_000, label: "Web Wireshark start" });
  await page.getByRole("dialog").getByRole("button", { name: "Close" }).click().catch(() => {});
  await page.locator(".sim-link").first().click({ button: "right" });
  await page.getByRole("menuitem", { name: "Open Web Wireshark (Inline)" }).click();
  const frame = page.getByTitle("Web Wireshark");
  await frame.waitFor();
  await frame.contentFrame().locator("canvas").waitFor({ timeout: 60_000 });
  await page.locator(".sim-link").first().click({ button: "right" });
  await page.getByRole("menuitem", { name: "Stop capture" }).click();
  await frame.waitFor({ state: "detached" });
  assert.deepEqual(ui.errors, []);
});
