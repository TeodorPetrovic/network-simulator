import test from "node:test";
import assert from "node:assert/strict";
import { startServer, newProject, launchPage, openProject, templateByType, addNode, waitFor } from "./harness.mjs";

// Nodes in every quadrant of the (negative-capable) GNS3 coordinate plane must drag freely in every
// direction without being clamped to a 0..scene box, and without snapping when snap-to-grid is off.
test("nodes drag freely across all four quadrants without snapping", async (t) => {
  const server = await startServer();
  const ui = await launchPage(server);
  t.after(async () => { await ui.close(); await server.stop(); });
  const pid = await newProject(server, "canvas-move");
  const vpcs = await templateByType(server, "vpcs");
  const spots = [[300, -200], [-300, -200], [-300, 200], [300, 200]];
  const nodes = [];
  for (const [i, [x, y]] of spots.entries()) nodes.push(await addNode(server, pid, vpcs, x, y, `N${i + 1}`));
  await openProject(server, ui.page, pid);
  const page = ui.page;
  await page.getByLabel("Fit to window").click().catch(() => {});
  const positions = async () => Object.fromEntries((await server.api("GET", `/projects/${pid}/nodes`)).json.map((n) => [n.name, [n.x, n.y]]));
  const before = await positions();
  const deltas = [[-137, 91], [211, 63], [113, -157], [-97, -83]];
  for (const [i, [dx, dy]] of deltas.entries()) {
    const box = await page.getByLabel(`Node N${i + 1}`).boundingBox();
    const scale = box.width / 80; // nodes render ~80 scene units wide at 100% zoom
    await page.mouse.move(box.x + box.width / 2, box.y + box.height / 2);
    await page.mouse.down();
    await page.mouse.move(box.x + box.width / 2 + dx * 0.5, box.y + box.height / 2 + dy * 0.5, { steps: 6 });
    await page.mouse.move(box.x + box.width / 2 + dx, box.y + box.height / 2 + dy, { steps: 6 });
    await page.mouse.up();
    await waitFor(async () => { const now = (await positions())[`N${i + 1}`]; return now[0] !== before[`N${i + 1}`][0]; }, { label: `N${i + 1} moved` });
  }
  const after = await positions();
  for (const [i, [dx, dy]] of deltas.entries()) {
    const [bx, by] = before[`N${i + 1}`], [ax, ay] = after[`N${i + 1}`];
    const tolerance = 12; // pixel-to-scene rounding only; a clamp or a 10-unit grid would exceed this on at least one axis
    const movedX = ax - bx, movedY = ay - by;
    assert.ok(Math.sign(movedX) === Math.sign(dx) && Math.sign(movedY) === Math.sign(dy), `N${i + 1} moved the wrong way: ${movedX},${movedY} for ${dx},${dy}`);
    assert.ok(Math.abs(Math.abs(movedX) - Math.abs(dx)) / Math.abs(dx) < 0.35 && Math.abs(Math.abs(movedY) - Math.abs(dy)) / Math.abs(dy) < 0.35, `N${i + 1} moved ${movedX},${movedY} but the pointer moved about ${dx},${dy} px (tolerance ${tolerance})`);
  }
  assert.deepEqual(ui.errors, []);
});
