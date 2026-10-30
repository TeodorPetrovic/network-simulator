import test from "node:test";
import assert from "node:assert/strict";
import { startServer, newProject, launchPage, openProject, addNode, templateByType, waitFor } from "./harness.mjs";

async function setup(t, name) {
  const server = await startServer();
  const ui = await launchPage(server);
  const projectId = await newProject(server, name);
  t.after(async () => {
    await ui.close();
    const log = server.log();
    await server.stop();
    assert.deepEqual(ui.errors, []);
    assert.doesNotMatch(log, /database is locked|\b500\b/i);
  });
  await openProject(server, ui.page, projectId);
  return { server, page: ui.page, projectId };
}

async function nodes(server, projectId) {
  const response = await server.api("GET", `/projects/${projectId}/nodes`);
  assert.equal(response.status, 200);
  return response.json;
}

test("canvas layout aligns and distributes selected nodes; Ctrl+A and arrow nudges update positions", async (t) => {
  const { server, page, projectId } = await setup(t, "canvas-layout");
  const template = await templateByType(server, "vpcs");
  await addNode(server, projectId, template, 0, 0, "Layout A");
  await addNode(server, projectId, template, 200, 100, "Layout B");
  await addNode(server, projectId, template, 400, 300, "Layout C");
  await page.reload();
  await page.getByRole("group", { name: "Network topology canvas" }).focus();
  await page.keyboard.press("Control+a");
  await page.getByRole("menuitem", { name: "Arrange" }).click();
  await page.getByRole("menuitem", { name: "Align horizontal centers" }).click();
  await waitFor(async () => {
    const current = await nodes(server, projectId);
    return new Set(current.map((node) => node.y)).size === 1;
  }, { label: "horizontal alignment" });
  await page.getByRole("menuitem", { name: "Arrange" }).click();
  await page.getByRole("menuitem", { name: "Distribute horizontally" }).click();
  await waitFor(async () => {
    const current = (await nodes(server, projectId)).sort((a, b) => a.x - b.x);
    return current.length === 3 && Math.abs((current[1].x - current[0].x) - (current[2].x - current[1].x)) < 0.01;
  }, { label: "horizontal distribution" });
  await page.getByRole("group", { name: "Network topology canvas" }).focus();
  await page.keyboard.press("ArrowRight");
  await waitFor(async () => (await nodes(server, projectId)).some((node) => node.x > 400), { label: "arrow key nudge" });
});

test("copy and paste duplicates selected nodes and preserves their internal links", async (t) => {
  const { server, page, projectId } = await setup(t, "canvas-copy-paste");
  const template = await templateByType(server, "vpcs");
  const first = await addNode(server, projectId, template, 0, 0, "Copy A");
  const second = await addNode(server, projectId, template, 200, 0, "Copy B");
  const created = await server.api("POST", `/projects/${projectId}/links`, { nodes: [
    { node_id: first.node_id, adapter_number: 0, port_number: 0 },
    { node_id: second.node_id, adapter_number: 0, port_number: 0 },
  ], link_type: "ethernet" });
  assert.equal(created.status, 201);
  await page.reload();
  await page.getByRole("group", { name: "Network topology canvas" }).focus();
  await page.keyboard.press("Control+a");
  await page.keyboard.press("Control+c");
  await page.keyboard.press("Control+v");
  await waitFor(async () => (await nodes(server, projectId)).length === 4, { label: "pasted nodes" });
  await waitFor(async () => {
    const links = await server.api("GET", `/projects/${projectId}/links`);
    return links.status === 200 && links.json.length === 2;
  }, { label: "pasted internal link" });
  const allNodes = await nodes(server, projectId);
  const pasted = allNodes.filter((node) => node.name.includes("copy"));
  assert.equal(pasted.length, 2);
});

test("selected nodes can be locked, and deleting a selection asks for confirmation", async (t) => {
  const { server, page, projectId } = await setup(t, "canvas-lock-delete");
  const template = await templateByType(server, "vpcs");
  const first = await addNode(server, projectId, template, 0, 0, "Lock A");
  await addNode(server, projectId, template, 200, 0, "Lock B");
  await page.reload();
  await page.getByRole("group", { name: "Network topology canvas" }).focus();
  await page.keyboard.press("Control+a");
  await page.getByRole("menuitem", { name: "Arrange" }).click();
  await page.getByRole("menuitem", { name: "Lock selected" }).click();
  await waitFor(async () => (await nodes(server, projectId)).every((node) => node.locked === true), { label: "locked selection" });
  const before = (await nodes(server, projectId)).find((node) => node.node_id === first.node_id);
  await page.keyboard.press("ArrowRight");
  await page.waitForTimeout(200);
  assert.equal((await nodes(server, projectId)).find((node) => node.node_id === first.node_id).x, before.x);
  await page.getByRole("menuitem", { name: "Arrange" }).click();
  await page.getByRole("menuitem", { name: "Unlock selected" }).click();
  await waitFor(async () => (await nodes(server, projectId)).every((node) => !node.locked), { label: "unlocked selection" });
  let dialogMessage = "";
  page.once("dialog", async (dialog) => { dialogMessage = dialog.message(); await dialog.accept(); });
  await page.getByRole("group", { name: "Network topology canvas" }).focus();
  await page.keyboard.press("Delete");
  assert.match(dialogMessage, /2 selected devices/);
  await waitFor(async () => (await nodes(server, projectId)).length === 0, { label: "confirmed delete selection" });
});

test("custom symbol upload uses the project symbol endpoint and can be applied", async (t) => {
  const { server, page, projectId } = await setup(t, "canvas-symbol-upload");
  const template = await templateByType(server, "vpcs");
  const node = await addNode(server, projectId, template, 0, 0, "Symbol PC");
  await page.reload();
  await page.locator(`[aria-label="Node ${node.name}"]`).click({ button: "right" });
  await page.getByRole("menuitem", { name: "Change symbol" }).click();
  await page.getByLabel("Upload custom symbol").setInputFiles({ name: "custom-test.svg", mimeType: "image/svg+xml", buffer: Buffer.from('<svg xmlns="http://www.w3.org/2000/svg" width="32" height="32"><circle cx="16" cy="16" r="12"/></svg>') });
  await waitFor(async () => (await server.api("GET", "/symbols")).json.some((item) => item.symbol_id === "custom-test.svg"), { label: "uploaded custom symbol" });
  await page.getByRole("combobox", { name: "Node symbol" }).click();
  await page.getByRole("option", { name: "custom-test.svg" }).click();
  await page.getByRole("button", { name: "Apply symbol" }).click();
  await waitFor(async () => (await nodes(server, projectId)).find((item) => item.node_id === node.node_id)?.symbol === "custom-test.svg", { label: "custom symbol applied" });
  await page.locator(`[aria-label="Node ${node.name}"] .sim-node-symbol[href^="blob:"]`).waitFor();
  const symbols = await server.api("GET", "/symbols");
  assert.ok(symbols.json.some((item) => item.symbol_id === "custom-test.svg"));
});
