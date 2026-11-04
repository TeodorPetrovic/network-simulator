import test from "node:test";
import assert from "node:assert/strict";
import { startServer, launchPage } from "./harness.mjs";
test("appliance library body is laid out and import is clickable", async (t) => {
  const server = await startServer(); const ui = await launchPage(server);
  t.after(async () => { await ui.close(); await server.stop(); });
  await ui.page.goto(server.base + "/appliances");
  const b = ui.page.getByRole("button", { name: /Import .gns3a/ }).first();
  await b.waitFor();
  await b.click({ timeout: 4000 });
  assert.deepEqual(ui.errors, []);
});
