// The device sidebar must not shift the map when a category opens, and dropping on it must not add a device.
import test from "node:test";
import assert from "node:assert/strict";
import { startServer, newProject, launchPage, openProject, addNode, templateByType } from "./harness.mjs";

test("opening a device category overlays the map without moving it", async (t) => {
  const server = await startServer();
  const ui = await launchPage(server);
  const projectId = await newProject(server, "sidebar-layout");
  t.after(async () => { await ui.close(); await server.stop(); });
  const template = await templateByType(server, "vpcs");
  await addNode(server, projectId, template, 0, 0, "PC1");
  await openProject(server, ui.page, projectId);
  const page = ui.page;
  const box = (sel) => page.locator(sel).first().boundingBox();
  const before = { canvas: await box(".sim-canvas-card"), rail: await box(".sim-device-rail"), strip: await box(".sim-device-rail-buttons") };
  await page.screenshot({ path: "/tmp/sidebar-before.png" });
  await page.getByRole("button", { name: "End devices" }).click();
  await page.locator(".sim-device-panel").waitFor();
  await page.waitForTimeout(400);
  const after = { canvas: await box(".sim-canvas-card"), rail: await box(".sim-device-rail"), strip: await box(".sim-device-rail-buttons"), panel: await box(".sim-device-panel") };
  await page.screenshot({ path: "/tmp/sidebar-after.png" });
  console.log(JSON.stringify({ before, after }));
  assert.equal(after.canvas.x, before.canvas.x, "map must not move");
  assert.equal(after.canvas.width, before.canvas.width, "map must not resize");
  assert.equal(after.rail.width, before.rail.width, "rail must keep its width");
  assert.equal(after.strip.width, before.strip.width);

  // Dragging a template and dropping it back on the sidebar must not add a device.
  const count = async () => (await server.api("GET", `/projects/${projectId}/nodes`)).json.length;
  assert.equal(await count(), 1);
  await page.locator(".sim-device-template").first().dragTo(page.locator(".sim-device-panel-list"));
  await page.waitForTimeout(800);
  assert.equal(await count(), 1, "dropping on the sidebar must not add a device");
  await page.locator(".sim-device-template").first().dragTo(page.locator(".sim-canvas-surface"), { targetPosition: { x: 600, y: 200 } });
  await page.waitForTimeout(1500);
  assert.equal(await count(), 2, "dropping on the map adds one");
});
