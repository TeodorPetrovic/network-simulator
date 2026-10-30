import test from "node:test";
import assert from "node:assert/strict";
import { startServer, launchPage, newProject, waitFor } from "./harness.mjs";

async function fixture(t, viewport) {
  const server = await startServer();
  const ui = await launchPage(server, viewport ? { width: viewport[0], height: viewport[1] } : {});
  t.after(async () => { assert.deepEqual(ui.errors, []); await ui.close(); await server.stop(); });
  return { server, base: server.base, ...ui };
}

test("all non-workspace navigation pages render inside the shared app shell", async (t) => {
  const { page, base } = await fixture(t);
  const pages = [
    ["Projects", "/"], ["Preferences", "/preferences"], ["Appliances", "/appliances"],
    ["Servers", "/computes"], ["Settings", "/settings"], ["Access", "/access/users"],
    ["My account", "/user-settings"], ["System status", "/system-status"], ["Installed software", "/installed-software"],
  ];
  for (const [label, path] of pages) {
    await page.goto(base + path);
    await page.getByRole("navigation", { name: "Application pages" }).waitFor();
    await page.getByRole("link", { name: label, exact: true }).waitFor();
    assert.ok(await page.locator("h1").first().isVisible(), `${label} should have a visible heading`);
    assert.ok((await page.locator("body").innerText()).trim().length > 30, `${label} should not be blank`);
  }
});

test("projects create, rename, duplicate, export, refresh, sort, close, and delete through UI and REST", async (t) => {
  const { server, page } = await fixture(t);
  await page.goto(server.base);
  await page.getByLabel("New project name").fill("Page Alpha");
  await page.getByRole("button", { name: "Create project" }).click();
  await page.waitForURL(/\/projects\//);
  const projects = (await server.api("GET", "/projects")).json;
  const alpha = projects.find((p) => p.name === "Page Alpha");
  assert.ok(alpha);
  await page.goto(server.base);
  await page.getByRole("button", { name: "Close project" }).click();
  assert.equal((await server.api("GET", `/projects/${alpha.project_id}`)).json.status, "closed");
  await page.getByRole("button", { name: `Settings for Page Alpha` }).click();
  const projectSettings = page.getByRole("dialog", { name: "Project settings" });
  await projectSettings.getByLabel("Show layers").check();
  await page.locator("#project-name").fill("Page Renamed");
  await projectSettings.getByRole("button", { name: "Apply" }).click();
  assert.equal((await server.api("GET", `/projects/${alpha.project_id}`)).json.name, "Page Renamed");
  assert.equal((await server.api("GET", `/projects/${alpha.project_id}`)).json.show_layers, true);
  await page.getByRole("button", { name: "Duplicate Page Renamed" }).click();
  await page.locator("#duplicate-project-name").fill("Page Copy");
  await page.getByRole("button", { name: "Duplicate" }).click();
  assert.ok((await server.api("GET", "/projects")).json.some((p) => p.name === "Page Copy"));
  await page.getByRole("button", { name: "Export Page Renamed" }).click();
  await page.getByRole("button", { name: "Refresh projects" }).click();
  for (const heading of ["Project", "Status", "Created"]) await page.getByRole("button", { name: new RegExp(heading) }).click();
  page.once("dialog", (dialog) => dialog.accept());
  await page.getByRole("button", { name: "Delete Page Renamed" }).click();
  await page.getByRole("button", { name: "Delete Page Renamed" }).waitFor({ state: "detached" });
  assert.deepEqual((await server.api("GET", "/projects")).json.map((p) => p.name).includes("Page Renamed"), false);
});

test("compute CRUD uses the compute REST resource and protects local compute", async (t) => {
  const { server, page } = await fixture(t);
  await page.goto(server.base + "/computes");
  await page.getByRole("button", { name: "Add compute" }).click();
  const dialog = page.getByRole("dialog", { name: "Add compute" });
  await dialog.getByLabel("Name").fill("Remote E2E");
  await dialog.getByLabel("Host").fill("127.0.0.1");
  await dialog.getByLabel("Compute ID").fill("remote-e2e");
  await dialog.getByRole("button", { name: "Save" }).click();
  const compute = (await server.api("GET", "/computes")).json.find((c) => c.name === "Remote E2E");
  assert.ok(compute);
  const row = page.getByRole("row").filter({ hasText: "Remote E2E" });
  await row.getByRole("button", { name: "Edit" }).click();
  await page.getByRole("dialog", { name: "Edit compute" }).getByLabel("Name").fill("Remote Updated");
  await page.getByRole("button", { name: "Save" }).click();
  assert.ok((await server.api("GET", "/computes")).json.some((c) => c.name === "Remote Updated"));
  page.once("dialog", (d) => d.accept());
  await page.getByRole("row").filter({ hasText: "Remote Updated" }).getByRole("button", { name: "Delete" }).click();
  assert.ok(!(await server.api("GET", "/computes")).json.some((c) => c.name === "Remote Updated"));
  const local = page.getByRole("row").filter({ hasText: "local" });
  assert.equal(await local.getByRole("button", { name: "Delete" }).isDisabled(), true);
});

for (const [width, height] of [[1500, 900], [1100, 700]]) test(`application layout fits ${width}x${height} without horizontal page scroll`, async (t) => {
  const { server, page } = await fixture(t, [width, height]);
  await page.goto(server.base);
  await page.getByRole("navigation", { name: "Application pages" }).waitFor();
  assert.equal(await page.evaluate(() => document.documentElement.scrollWidth <= innerWidth), true);
  const card = await page.locator(".projects-table-card").boundingBox();
  const header = await page.locator(".sim-app-header").boundingBox();
  const tableHead = await page.locator(".projects-table-card thead").boundingBox();
  assert.ok(header && card && tableHead);
  assert.ok(tableHead.y - card.y <= 2, "table header should start at card top");
});
