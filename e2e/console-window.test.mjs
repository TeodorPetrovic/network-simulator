import test from "node:test";
import assert from "node:assert/strict";
import { startServer, newProject, launchPage, openProject, templateByType, addNode, waitFor } from "./harness.mjs";

test("floating console drags across the whole screen and comes to front when clicked", async (t) => {
  const server = await startServer();
  const ui = await launchPage(server, { width: 2000, height: 1000 });
  t.after(async () => { await ui.close(); await server.stop(); });
  const pid = await newProject(server, "console-window");
  const pc1 = await addNode(server, pid, await templateByType(server, "vpcs"), 0, 0, "PC1");
  const pc2 = await addNode(server, pid, await templateByType(server, "vpcs"), 300, 0, "PC2");
  const wire = await server.api("POST", `/projects/${pid}/links`, { nodes: [{ node_id: pc1.node_id, adapter_number: 0, port_number: 0 }, { node_id: pc2.node_id, adapter_number: 0, port_number: 0 }] });
  assert.equal(wire.status, 201);
  await openProject(server, ui.page, pid);
  const page = ui.page;
  await page.locator('[aria-label="Node PC1"]').waitFor();
  await page.getByRole("button", { name: "Start all" }).first().click();
  await waitFor(async () => (await server.api("GET", `/projects/${pid}/nodes`)).json.every((n) => n.status === "started"), { label: "started" });
  await page.locator('[aria-label="Node PC1"]').click({ button: "right" });
  await page.getByRole("menuitem", { name: "Open console" }).click();
  const dock = page.getByRole("region", { name: "Device consoles" });
  await dock.waitFor();
  const box = async () => (await dock.boundingBox());
  const dragBy = async (grab, to) => { await page.mouse.move(grab.x, grab.y); await page.mouse.down(); await page.mouse.move(to.x, to.y, { steps: 15 }); await page.mouse.up(); };
  // expanded window: grab the empty header area and move it across the screen
  for (const targetX of [100, 500, 1100]) {
    const before = await box();
    await dragBy({ x: before.x + before.width - 200, y: before.y + 21 }, { x: targetX + before.width - 200, y: before.y + 61 });
    const after = await box();
    assert.ok(Math.abs(after.x - targetX) < 6, `expanded: dragged to x=${targetX}, ended at ${after.x}`);
  }
  // minimized pill: grab the Console label and move it across the screen, including the left third
  await dock.getByRole("button", { name: "Minimize console dock" }).click();
  await page.waitForTimeout(600);
  for (const targetX of [100, 400, 650, 1200, 1800]) {
    const before = await box();
    const label = await dock.locator(".sim-console-dock-actions span").first().boundingBox();
    await dragBy({ x: label.x + 10, y: label.y + label.height / 2 }, { x: label.x + 10 + (targetX - before.x), y: label.y + label.height / 2 });
    const after = await box();
    assert.ok(Math.abs(after.x - targetX) < 6, `minimized: dragged to x=${targetX}, ended at ${after.x}`);
  }
  // capture viewer: resizable below the old 800x600 floor, and stacking follows the last click
  await dock.getByRole("button", { name: "Restore console dock" }).click();
  await server.api("POST", `/projects/${pid}/links/${wire.json.link_id}/capture/start`, { capture_file_name: "z.pcap", data_link_type: "DLT_EN10MB" });
  await page.locator(".sim-link").first().evaluate((el) => el.dispatchEvent(new MouseEvent("contextmenu", { bubbles: true, button: 2, clientX: 700, clientY: 300 })));
  await page.getByRole("menuitem", { name: "Open built-in Capture Viewer" }).click();
  const capture = page.locator(".capture-window");
  await capture.waitFor();
  const z = async (locator) => Number(await locator.evaluate((el) => getComputedStyle(el).zIndex));
  assert.ok(await z(capture) > await z(dock), "a freshly opened capture window starts on top");
  await dock.locator("header").click({ position: { x: 20, y: 20 } });
  assert.ok(await z(dock) > await z(capture), "clicking the console brings it above the capture window");
  const cbox = await capture.boundingBox();
  const grip = await capture.locator(".capture-window-resize").boundingBox();
  await page.mouse.move(grip.x + 10, grip.y + 10); await page.mouse.down();
  await page.mouse.move(grip.x + 10 - (cbox.width - 500), grip.y + 10 - (cbox.height - 320), { steps: 10 }); await page.mouse.up();
  const resized = await capture.boundingBox();
  assert.ok(resized.width < 560 && resized.height < 380, `capture window resized to ${resized.width}x${resized.height}`);
  assert.deepEqual(ui.errors, []);
});
