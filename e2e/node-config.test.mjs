import test from "node:test";
import assert from "node:assert/strict";
import { startServer, newProject, launchPage, openProject, templateByType, addNode, waitFor } from "./harness.mjs";

test("right-click Edit config saves a VPCS startup script", async (t) => {
  const server = await startServer();
  const ui = await launchPage(server);
  t.after(async () => { await ui.close(); await server.stop(); });
  const pid = await newProject(server, "edit-config");
  await addNode(server, pid, await templateByType(server, "vpcs"), 0, 0, "PC1");
  await openProject(server, ui.page, pid);
  const node = ui.page.locator('[aria-label="Node PC1"]');
  await node.waitFor();
  await node.click({ button: "right" });
  await ui.page.getByRole("menuitem", { name: "Edit config" }).click();
  await ui.page.getByRole("textbox", { name: "Configuration" }).fill("ip 10.0.0.2/24 10.0.0.1");
  await ui.page.getByRole("button", { name: "Save" }).click();
  const saved = await waitFor(async () => { const { json } = await server.api("GET", `/projects/${pid}/nodes`); return json[0].startup_script?.includes("10.0.0.2") && json[0]; }, { label: "script saved" });
  assert.match(saved.startup_script, /ip 10\.0\.0\.2\/24/);
  assert.deepEqual(ui.errors, []);
});
