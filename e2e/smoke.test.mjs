import test from "node:test";
import assert from "node:assert/strict";
import { startServer, newProject, launchPage, openProject, templateByType, addNode, waitFor } from "./harness.mjs";

test("workspace loads, VPCS starts from the toolbar and shows as started", async (t) => {
  const server = await startServer();
  const ui = await launchPage(server);
  t.after(async () => { await ui.close(); await server.stop(); });
  const pid = await newProject(server, "smoke");
  await addNode(server, pid, await templateByType(server, "vpcs"), 0, 0, "PC1");
  await openProject(server, ui.page, pid);
  await ui.page.locator('[aria-label="Node PC1"]').waitFor();
  await ui.page.getByRole("button", { name: "Start all" }).first().click();
  const nodes = await waitFor(async () => { const { json } = await server.api("GET", `/projects/${pid}/nodes`); return json.every((n) => n.status === "started") && json; }, { label: "node started" });
  assert.equal(nodes[0].status, "started");
  assert.deepEqual(ui.errors, []);
});
