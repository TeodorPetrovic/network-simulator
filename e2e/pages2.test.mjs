import test from "node:test";
import assert from "node:assert/strict";
import { startServer, launchPage, newProject, waitFor } from "./harness.mjs";

async function fixture(t) {
  const server = await startServer();
  const ui = await launchPage(server);
  t.after(async () => { assert.deepEqual(ui.errors, []); await ui.close(); await server.stop(); });
  return { server, ...ui };
}

test("Preferences runtime tabs include all built-ins and mark unsupported types", async (t) => {
  const { page, server } = await fixture(t);
  await page.goto(server.base + "/preferences");
  for (const label of ["Docker", "QEMU", "VPCS", "Dynamips (IOS routers)", "Cloud", "NAT", "Ethernet hub", "Ethernet switch"]) {
    await page.getByRole("button", { name: label, exact: true }).click();
    await page.getByRole("heading", { name: `${label} templates`, exact: true }).waitFor();
    await page.getByRole("button", { name: "Add template" }).waitFor();
  }
  for (const label of ["IOU", "VirtualBox", "VMware"]) {
    await page.getByRole("button", { name: new RegExp(label) }).click();
    await page.getByText(`${label} is unsupported`, { exact: true }).waitFor();
    assert.equal(await page.getByRole("button", { name: "Add template" }).count(), 0);
  }
});

test("Preferences creates templates for each supported runtime and exposes its fields", async (t) => {
  const { page, server } = await fixture(t);
  await page.goto(server.base + "/preferences");
  const types = [
    ["Docker", "Docker template", "Environment"], ["QEMU", "QEMU template", "RAM (MB)"],
    ["VPCS", "VPCS template", "Startup script"], ["Dynamips (IOS routers)", "Dynamips (IOS routers) template", "Platform"],
    ["Cloud", "Cloud template", "Ethernet interface"], ["NAT", "NAT template", "Template name"],
    ["Ethernet hub", "Ethernet hub template", "Number of ports"], ["Ethernet switch", "Ethernet switch template", "Port configuration (port number, VLAN, mode, EtherType)"],
  ];
  for (const [runtime, title, field] of types) {
    await page.getByRole("button", { name: runtime, exact: true }).click();
    await page.getByRole("button", { name: "Add template" }).click();
    const dialog = page.getByRole("dialog", { name: "Create template" });
    await dialog.getByLabel("Template name").fill(`E2E ${runtime}`);
    if (runtime === "Cloud") await dialog.getByRole("tab", { name: "Ethernet interfaces" }).click();
    if (runtime === "Ethernet switch") await dialog.getByRole("tab", { name: "Ports" }).click();
    await dialog.getByText(field, { exact: true }).waitFor();
    await dialog.getByRole("button", { name: "Create template" }).click();
    await page.getByText(`E2E ${runtime}`, { exact: true }).waitFor();
  }
  const created = (await server.api("GET", "/templates")).json;
  assert.ok(created.some((x) => x.name === "E2E Dynamips (IOS routers)"));
});

test("Dynamips platform selector shows stored value and all supported choices; save persists", async (t) => {
  const { page, server } = await fixture(t);
  const templates = (await server.api("GET", "/templates")).json;
  let dynamips = templates.find((x) => x.template_type === "dynamips");
  await page.goto(server.base + "/preferences");
  await page.getByRole("button", { name: "Dynamips (IOS routers)", exact: true }).click();
  if (!dynamips) {
    await page.getByRole("button", { name: "Add template" }).click();
    const create = page.getByRole("dialog", { name: "Create template" });
    await create.getByLabel("Template name").fill("Dynamips platform E2E");
    await create.getByRole("button", { name: "Create template" }).click();
    await waitFor(async () => (await server.api("GET", "/templates")).json.find((x) => x.name === "Dynamips platform E2E"), { label: "create Dynamips template" });
    dynamips = (await server.api("GET", "/templates")).json.find((x) => x.name === "Dynamips platform E2E");
  }
  await page.getByRole("row").filter({ hasText: dynamips.name }).getByRole("button", { name: "Configure" }).click();
  const dialog = page.getByRole("dialog", { name: "Template settings" });
  const platform = dialog.getByRole("combobox", { name: "Platform" });
  await platform.waitFor();
  await platform.click();
  for (const value of ["c1700", "c2600", "c2691", "c3600", "c3725", "c3745", "c7200"]) await page.getByRole("option", { name: value, exact: true }).waitFor();
  await page.getByRole("option", { name: "c3725", exact: true }).click();
  await dialog.getByRole("button", { name: "Save template" }).click();
  await waitFor(async () => (await dialog.getByRole("alert").count()) > 0 || (await server.api("GET", "/templates")).json.find((x) => x.template_id === dynamips.template_id)?.platform === "c3725", { label: "Dynamips platform save response" });
  assert.equal(await dialog.getByRole("alert").count(), 0);
  await waitFor(async () => (await server.api("GET", "/templates")).json.find((x) => x.template_id === dynamips.template_id)?.platform === "c3725", { label: "Dynamips platform persistence" });
});

test("template configure, duplicate and delete flow use REST", async (t) => {
  const { page, server } = await fixture(t);
  await page.goto(server.base + "/preferences");
  await page.getByRole("button", { name: "Docker", exact: true }).click();
  await page.getByRole("button", { name: "Add template" }).click();
  const create = page.getByRole("dialog", { name: "Create template" });
  await create.getByLabel("Template name").fill("Docker E2E Original");
  await create.getByRole("button", { name: "Create template" }).click();
  await waitFor(async () => (await server.api("GET", "/templates")).json.find((x) => x.name === "Docker E2E Original"), { label: "create Docker template" });
  const seeded = (await server.api("GET", "/templates")).json.find((x) => x.name === "Docker E2E Original");
  const row = page.getByRole("row").filter({ hasText: seeded.name });
  await row.getByRole("button", { name: "Configure" }).click();
  const dialog = page.getByRole("dialog", { name: "Template settings" });
  await dialog.getByLabel("Template name").fill("E2E Configured Docker");
  for (const label of ["Template name", "Start command", "Environment", "Extra hosts", "Persistent paths"]) await dialog.getByLabel(label).waitFor();
  await dialog.getByRole("tab", { name: "Network" }).click();
  await dialog.getByLabel("Adapters").waitFor();
  await dialog.getByRole("tab", { name: "Advanced" }).click();
  for (const label of ["Memory limit (MB)", "CPU limit"]) await dialog.getByLabel(label).waitFor();
  await dialog.getByRole("button", { name: "Save template" }).click();
  await waitFor(async () => (await server.api("GET", "/templates")).json.some((x) => x.template_id === seeded.template_id && x.name === "E2E Configured Docker"), { label: "template configure" });
  await page.getByRole("button", { name: "Duplicate E2E Configured Docker" }).click();
  await waitFor(async () => (await server.api("GET", "/templates")).json.length > 1, { label: "template duplicate" });
  const copy = (await server.api("GET", "/templates")).json.find((x) => x.template_id !== seeded.template_id && x.template_type === "docker");
  assert.ok(copy);
  page.once("dialog", (d) => d.accept());
  await page.getByRole("button", { name: `Delete ${copy.name}` }).click();
  await waitFor(async () => !(await server.api("GET", "/templates")).json.some((x) => x.template_id === copy.template_id), { label: "template delete" });
});

test("appliance library empty search and marketplace catalog search are usable offline", async (t) => {
  const { page, server } = await fixture(t);
  await page.goto(server.base + "/appliances");
  await page.getByRole("heading", { name: "Appliances" }).waitFor();
  const search = page.getByRole("textbox", { name: "Filter installed appliances" });
  await search.fill("no such appliance");
  assert.equal(await search.inputValue(), "no such appliance");
  await page.goto(server.base);
  await page.getByRole("button", { name: "Image catalog" }).click();
  const catalog = page.getByRole("dialog");
  await catalog.getByRole("textbox", { name: "Search catalog" }).fill("network-simulator-offline-e2e");
  await catalog.getByRole("button", { name: "Search", exact: true }).click();
  await catalog.getByText(/Search the GNS3 appliance registry|catalog unavailable|could not/i).waitFor({ timeout: 15000 });
  assert.ok(await catalog.isVisible());
});

test("access management explains unauthenticated local mode", async (t) => {
  const { page, server } = await fixture(t);
  for (const path of ["/access/users", "/access/groups", "/access/roles"]) {
    await page.goto(server.base + path);
    await page.locator("h1").first().waitFor();
    assert.ok((await page.locator("body").innerText()).length > 30);
  }
});

test("settings save, system status and installed software render", async (t) => {
  const { page, server } = await fixture(t);
  await page.goto(server.base + "/settings");
  await page.getByRole("heading", { name: "Settings" }).waitFor();
  await page.getByRole("button", { name: "Save settings" }).click();
  await page.getByRole("status").getByText("Controller settings saved.").waitFor();
  for (const path of ["/system-status", "/installed-software"]) {
    await page.goto(server.base + path);
    await page.locator("h1").first().waitFor();
    assert.ok((await page.locator("body").innerText()).length > 30);
  }
});

test("project snapshots create, restore and delete from File menu", async (t) => {
  const { page, server } = await fixture(t);
  const id = await newProject(server, "Snapshot E2E");
  await page.goto(`${server.base}/projects/${id}`);
  await page.locator(".sim-workspace").waitFor();
  await page.getByRole("menubar").getByText("File").click();
  await page.getByText("Snapshots…", { exact: true }).click();
  const dialog = page.getByRole("dialog");
  await dialog.getByRole("textbox").fill("E2E checkpoint");
  await dialog.getByRole("button", { name: "Create snapshot" }).click();
  await dialog.getByText("E2E checkpoint").waitFor();
  assert.equal((await server.api("GET", `/projects/${id}/snapshots`)).status, 200);
  await dialog.getByRole("button", { name: /restore/i }).click();
  page.once("dialog", (d) => d.accept());
  await dialog.getByRole("button", { name: /delete/i }).click();
});

test("project Info and Help dialogs open from toolbar", async (t) => {
  const { page, server } = await fixture(t);
  const id = await newProject(server, "Dialogs E2E");
  await page.goto(`${server.base}/projects/${id}`);
  await page.locator(".sim-workspace").waitFor();
  await page.getByRole("menubar").getByText("Help").click();
  await page.getByText("Keyboard shortcuts", { exact: true }).click();
  await page.getByRole("dialog").waitFor();
  await page.keyboard.press("Escape");
  await page.getByRole("menubar").getByText("Help").click();
  await page.getByText("About…", { exact: true }).click();
  await page.getByRole("dialog", { name: "Project information" }).waitFor();
});
