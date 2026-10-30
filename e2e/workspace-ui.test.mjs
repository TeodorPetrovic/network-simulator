import test from "node:test";
import assert from "node:assert/strict";
import { startServer, newProject, launchPage, openProject, templateByType, addNode, waitFor } from "./harness.mjs";

async function workspace(t, { nodes = [], name = "workspace-ui" } = {}) {
  const server = await startServer();
  const ui = await launchPage(server);
  t.after(async () => { const errors = [...ui.errors]; await ui.close(); await server.stop(); assert.deepEqual(errors, []); });
  const pid = await newProject(server, name);
  for (const n of nodes) await addNode(server, pid, await templateByType(server, n.type), n.x ?? 0, n.y ?? 0, n.name);
  await openProject(server, ui.page, pid);
  return { server, ui, page: ui.page, pid };
}
const button = (page, name) => page.getByRole("button", { name, exact: true });
const menu = (page, name) => page.getByRole("menuitem", { name, exact: true });
async function openMenu(page, name) { await page.getByRole("menuitem", { name, exact: true }).click(); }
async function finish(ui) { assert.deepEqual(ui.errors, []); }
async function count(page, selector) { return page.locator(selector).count(); }
async function zoom(page) { return Number((await page.locator(".sim-zoom-hint button").innerText()).replace("%", "")); }

test("right-click Configure opens the device editor for every available node type", async (t) => {
  const server = await startServer();
  const ui = await launchPage(server);
  t.after(async () => { const errors = [...ui.errors]; await ui.close(); await server.stop(); assert.deepEqual(errors, []); });
  const pid = await newProject(server, "configure-context-menu-types");
  const templates = (await server.api("GET", "/templates")).json;
  const wanted = ["ethernet_switch", "ethernet_hub", "cloud", "nat", "vpcs", "dynamips", "qemu", "docker"];
  const selected = wanted.map((type) => templates.find((template) => template.template_type === type)).filter(Boolean);
  for (const type of ["ethernet_switch", "ethernet_hub", "cloud", "nat", "vpcs"]) {
    assert.ok(selected.some((template) => template.template_type === type), `expected built-in ${type} template`);
  }
  for (const [index, template] of selected.entries()) {
    await addNode(server, pid, template, ((index % 3) - 1) * 180, (Math.floor(index / 3) - 1) * 150, `Configure${index}`);
  }
  await openProject(server, ui.page, pid);
  for (const [index, template] of selected.entries()) {
    const node = ui.page.locator(`.sim-node[aria-label="Node Configure${index}"]`);
    await node.waitFor();
    await node.click({ button: "right" });
    await menu(ui.page, "Configure device").click();
    const dialog = ui.page.getByRole("dialog", { name: `Configure Configure${index}` });
    await dialog.waitFor();
    const fields = {
      qemu: ["RAM (MB)", "vCPUs", "Network adapters", "CD-ROM image", "Boot priority", "Console type", "Additional QEMU options"],
      dynamips: ["RAM (MB)", "NVRAM (KB)", "Idle-PC", "Slot 0", "WIC 0", "Startup configuration"],
      docker: ["Docker image", "Start command", "Environment variables", "Network adapters"],
      vpcs: ["Console type", "Startup script"],
      cloud: ["Ethernet interfaces"],
      nat: ["Allowed host interfaces"],
      ethernet_switch: ["Port count", "Port VLANs", "Port VLAN types"],
      ethernet_hub: ["Port count", "Port names"],
    }[template.template_type] ?? [];
    for (const label of fields) await dialog.getByText(label, { exact: true }).waitFor();
    await ui.page.getByRole("button", { name: "Cancel", exact: true }).click();
  }
  assert.deepEqual(ui.errors, []);
});

// File menu: disabled entries are intentionally unavailable; enabled entries have visible effects.
test("File menu enabled actions open snapshots and return to projects; unavailable actions stay disabled", async (t) => {
  const { page, ui } = await workspace(t);
  await page.getByRole("menuitem", { name: "File" }).click();
  for (const label of ["New blank project", "Open project…", "Save project as…", "Export portable project…", "Import portable project…"]) assert.equal(await menu(page, label).isDisabled(), true, label);
  await menu(page, "Snapshots…").click();
  await page.getByRole("dialog", { name: "Project snapshots" }).waitFor();
  await page.keyboard.press("Escape");
  await page.getByRole("menuitem", { name: "File" }).click();
  await menu(page, "Close project / Back to projects").click();
  await page.getByRole("heading", { name: "Projects" }).waitFor();
  await finish(ui);
});

test("Edit menu selects all nodes, clears selection, and Preferences opens settings", async (t) => {
  const { page, ui } = await workspace(t, { nodes: [{ type: "vpcs", name: "SelectPC1" }, { type: "vpcs", name: "SelectPC2", x: 200 }] });
  await page.getByRole("menuitem", { name: "Edit" }).click();
  assert.equal(await menu(page, "Select all").isDisabled(), false);
  assert.equal(await menu(page, "Select none").isDisabled(), true);
  await menu(page, "Select all").click();
  assert.equal(await page.locator(".sim-node.is-selected").count(), 2);
  await page.getByRole("menuitem", { name: "Edit" }).click();
  assert.equal(await menu(page, "Select none").isDisabled(), false);
  await menu(page, "Select none").click();
  assert.equal(await page.locator(".sim-node.is-selected").count(), 0);
  await page.getByRole("menuitem", { name: "Edit" }).click();
  await menu(page, "Preferences…").click();
  await page.getByRole("heading", { name: "Preferences" }).waitFor();
  await finish(ui);
});

test("View menu controls change zoom, labels, grid, snap, lock, console dock, and summary docks", async (t) => {
  const { page, ui } = await workspace(t);
  await page.getByRole("menuitem", { name: "View" }).click();
  assert.equal(await menu(page, /Topology summary ✓/).isDisabled(), false);
  const z0 = await zoom(page);
  await menu(page, new RegExp(`Zoom in · ${z0}%`)).click();
  assert.equal(await zoom(page), z0 + 10);
  await page.getByRole("menuitem", { name: "View" }).click(); await menu(page, "Zoom out").click();
  assert.equal(await zoom(page), z0);
  await page.getByRole("menuitem", { name: "View" }).click(); await menu(page, "Reset zoom").click();
  assert.equal(await zoom(page), 100);
  await page.getByRole("menuitem", { name: "View" }).click(); await menu(page, "Fit to window").click();
  await page.getByRole("menuitem", { name: "View" }).click(); await menu(page, "Zoom to selection").click();
  await page.getByRole("menuitem", { name: "View" }).click(); await menu(page, "Show interface labels").click();
  await page.getByRole("menuitem", { name: "View" }).click(); await menu(page, "Show grid").click();
  await page.getByRole("menuitem", { name: "View" }).click(); await menu(page, /Snap to grid/).click();
  await page.getByRole("menuitem", { name: "View" }).click(); await menu(page, "Lock project").click();
  await page.getByRole("menuitem", { name: "View" }).click(); await menu(page, "Unlock project").waitFor(); await menu(page, "Unlock project").click();
  await page.getByRole("menuitem", { name: "View" }).click(); await menu(page, "Show Console dock").click();
  await page.getByRole("region", { name: "Device consoles" }).waitFor();
  await finish(ui);
});

test("Control menu enabled and disabled lifecycle actions reflect node state", async (t) => {
  const { server, page, ui, pid } = await workspace(t, { nodes: [{ type: "vpcs", name: "PC1" }] });
  await page.getByRole("menuitem", { name: "Control" }).click();
  await menu(page, "Start all").click();
  await waitFor(async () => (await server.api("GET", `/projects/${pid}/nodes`)).json[0]?.status === "started", { label: "node started" });
  await page.getByRole("menuitem", { name: "Control" }).click(); assert.equal(await menu(page, "Suspend all").isDisabled(), true);
  await page.keyboard.press("Escape"); await page.getByRole("menuitem", { name: "Control" }).click(); await menu(page, "Reload all").click();
  await waitFor(async () => (await server.api("GET", `/projects/${pid}/nodes`)).json[0]?.status === "started", { label: "node reloaded" });
  await page.getByRole("menuitem", { name: "Control" }).click(); await menu(page, "Stop all").click();
  await waitFor(async () => (await server.api("GET", `/projects/${pid}/nodes`)).json[0]?.status === "stopped", { label: "node stopped" });
  await page.getByRole("menuitem", { name: "Control" }).click(); assert.equal(await menu(page, "Console to all devices").isDisabled(), true);
  await finish(ui);
});

test("Node menu lifecycle, console, and delete controls follow selection", async (t) => {
  const { server, page, ui, pid } = await workspace(t, { nodes: [{ type: "vpcs", name: "PC1" }] });
  await page.getByRole("menuitem", { name: "Node" }).click();
  for (const label of ["Start selected", "Suspend selected", "Stop selected", "Reload selected", "Console selected", "Delete selected"]) assert.equal(await menu(page, label).isDisabled(), true, label);
  await button(page, "Node PC1").click();
  await page.getByRole("menuitem", { name: "Node" }).click(); await menu(page, "Start selected").click();
  await waitFor(async () => (await server.api("GET", `/projects/${pid}/nodes`)).json[0]?.status === "started", { label: "selected node started" });
  await page.getByRole("menuitem", { name: "Node" }).click(); assert.equal(await menu(page, "Suspend selected").isDisabled(), true);
  await page.keyboard.press("Escape"); await page.getByRole("menuitem", { name: "Node" }).click(); await menu(page, "Stop selected").click();
  await waitFor(async () => (await server.api("GET", `/projects/${pid}/nodes`)).json[0]?.status === "stopped", { label: "selected node stopped" });
  await page.getByRole("menuitem", { name: "Node" }).click(); await menu(page, "Start selected").click();
  await waitFor(async () => (await server.api("GET", `/projects/${pid}/nodes`)).json[0]?.status === "started", { label: "selected node restarted" });
  await page.getByRole("menuitem", { name: "Node" }).click(); await menu(page, "Reload selected").click();
  await waitFor(async () => (await server.api("GET", `/projects/${pid}/nodes`)).json[0]?.status === "started", { label: "selected node reloaded" });
  await page.getByRole("menuitem", { name: "Node" }).click(); await menu(page, "Console selected").click();
  await page.getByRole("region", { name: "Device consoles" }).waitFor();
  page.once("dialog", (dialog) => void dialog.accept());
  await page.getByRole("menuitem", { name: "Node" }).click(); await menu(page, "Delete selected").click();
  await page.getByRole("dialog").getByRole("button", { name: /Delete|Confirm/i }).click().catch(() => {});
  await waitFor(async () => (await server.api("GET", `/projects/${pid}/nodes`)).json.length === 0, { label: "selected node deleted" });
  await finish(ui);
});

test("Annotate menu tools enter drawing mode and expose conditional drawing actions", async (t) => {
  const { page, ui } = await workspace(t);
  for (const [label, tool] of [["Add a note / text", "text"], ["Add image", "image"], ["Rectangle", "rectangle"], ["Ellipse", "ellipse"], ["Line", "line"]]) {
    await page.getByRole("menuitem", { name: "Annotate" }).click(); await menu(page, label).click();
    const dialog = page.getByRole("dialog", { name: new RegExp(`Add ${tool} drawing`, "i") }); await dialog.waitFor();
    if (tool !== "image") { await dialog.getByRole("button", { name: "Choose appearance" }).click(); await page.getByText(new RegExp(`Click map to place ${tool}`)).waitFor(); await button(page, "Cancel").click(); }
    else await dialog.getByRole("button", { name: "Cancel" }).click();
  }
  await finish(ui);
});

test("Tools menu opens packet, image, appliance, screenshot, and log interfaces", async (t) => {
  const { page, ui, server, pid } = await workspace(t);
  await page.getByRole("menuitem", { name: "Tools" }).click(); assert.equal(await menu(page, "Packet capture…").isDisabled(), true);
  await menu(page, "Image catalog…").click(); await page.getByRole("dialog").waitFor(); await page.keyboard.press("Escape");
  await page.getByRole("menuitem", { name: "Tools" }).click(); await menu(page, "Appliances…").click(); await page.waitForURL("**/appliances**");
  await openProject(server, page, pid);
  await page.getByRole("menuitem", { name: "Tools" }).click(); await menu(page, "Project screenshot…").click(); await page.getByRole("dialog").waitFor(); await page.keyboard.press("Escape");
  await page.getByRole("menuitem", { name: "Tools" }).click(); await menu(page, "Project log…").click(); await page.getByRole("dialog").waitFor();
  await finish(ui);
});

test("Help menu opens shortcuts and About dialog", async (t) => {
  const { page, ui } = await workspace(t);
  await page.getByRole("menuitem", { name: "Help" }).click(); await menu(page, "Keyboard shortcuts").click();
  await page.getByRole("dialog").waitFor(); await page.keyboard.press("Escape");
  await page.getByRole("menuitem", { name: "Help" }).click(); await menu(page, "About…").click();
  await page.getByRole("dialog").waitFor(); await finish(ui);
});

test("every workspace icon toolbar action has a visible effect or is disabled", async (t) => {
  const { server, page, ui, pid } = await workspace(t, { nodes: [{ type: "vpcs", name: "ToolbarPC" }] });
  const names = ["Start all", "Suspend all", "Stop all", "Reload all", "Console all", "Show interface labels", "Zoom in", "Zoom out", "Fit to window", "Add note", "Add image", "Rectangle", "Ellipse", "Line", "Packet capture", "Draw link"];
  const toolbar = page.getByRole("toolbar", { name: "Workspace tools" });
  for (const name of names) assert.equal(await toolbar.getByRole("button", { name, exact: true }).count(), 1, name);
  assert.equal(await toolbar.getByRole("button", { name: "Packet capture", exact: true }).isDisabled(), true);
  const status = async () => (await server.api("GET", `/projects/${pid}/nodes`)).json[0]?.status;
  await toolbar.getByRole("button", { name: "Start all" }).click(); await waitFor(async () => await status() === "started", { label: "toolbar start" });
  await toolbar.getByRole("button", { name: "Console all" }).click(); await page.getByRole("region", { name: "Device consoles" }).waitFor();
  assert.equal(await toolbar.getByRole("button", { name: "Suspend all" }).isDisabled(), true);
  await toolbar.getByRole("button", { name: "Reload all" }).click();
  await waitFor(async () => await status() === "started", { label: "toolbar reload" });
  await toolbar.getByRole("button", { name: "Stop all" }).click(); await waitFor(async () => await status() === "stopped", { label: "toolbar stop" });
  let z = await zoom(page); await toolbar.getByRole("button", { name: "Zoom in" }).click(); assert.equal(await zoom(page), z + 10);
  await toolbar.getByRole("button", { name: "Zoom out" }).click(); assert.equal(await zoom(page), z);
  await toolbar.getByRole("button", { name: "Fit to window" }).click();
  await toolbar.getByRole("button", { name: "Show interface labels" }).click(); assert.equal(await toolbar.getByRole("button", { name: "Hide interface labels" }).getAttribute("aria-pressed"), "true");
  for (const [label, tool] of [["Add note", "text"], ["Add image", "image"], ["Rectangle", "rectangle"], ["Ellipse", "ellipse"], ["Line", "line"]]) {
    await toolbar.getByRole("button", { name: label }).click(); const dialog = page.getByRole("dialog", { name: new RegExp(`Add ${tool} drawing`, "i") }); await dialog.waitFor();
    if (tool !== "image") { await dialog.getByRole("button", { name: "Choose appearance" }).click(); await page.getByText(new RegExp(`Click map to place ${tool}`)).waitFor(); await button(page, "Cancel").click(); }
    else await dialog.getByRole("button", { name: "Cancel" }).click();
  }
  await toolbar.getByRole("button", { name: "Draw link" }).click(); await page.getByText("Select first endpoint").waitFor();
  await page.keyboard.press("Escape"); await page.getByRole("button", { name: "Draw link" }).waitFor();
  await finish(ui);
});

test("zoom hint percentage resets zoom and view zoom persists across reload", async (t) => {
  const { page, ui, pid } = await workspace(t);
  const z0 = await zoom(page); await button(page, "Zoom in").click(); assert.equal(await zoom(page), z0 + 10);
  await page.evaluate((id) => localStorage.setItem(`ns.view.${id}`, JSON.stringify({ zoom: 1.4, centerX: 0, centerY: 0 })), pid);
  await page.reload(); await page.locator(".sim-workspace").waitFor(); assert.equal(await zoom(page), 140);
  await page.locator(".sim-zoom-hint button").click(); assert.equal(await zoom(page), 100);
  await finish(ui);
});

test("device rail category flyouts filter and close, and click add creates a node", async (t) => {
  const { server, page, ui, pid } = await workspace(t);
  for (const cat of ["End devices", "Routers", "Switches", "Security devices", "All devices"]) {
    await button(page, cat).click(); await page.getByRole("region", { name: `${cat} templates` }).waitFor();
    assert.ok(await count(page, ".sim-device-template") > 0 || await page.getByText("No templates in this category.").count() === 1, `${cat} template list/empty state`);
    await button(page, "Close device list").click();
  }
  await button(page, "Routers").click();
  const row = page.locator(".sim-device-template").first(); const templateName = await row.locator("b").innerText();
  const filter = page.getByRole("textbox", { name: "Filter devices" }); await filter.fill("__no_such_template__"); await page.getByText("No templates in this category.").waitFor(); await filter.fill("");
  const before = (await server.api("GET", `/projects/${pid}/nodes`)).json.length;
  await row.getByRole("button", { name: `Add ${templateName}` }).click();
  await waitFor(async () => (await server.api("GET", `/projects/${pid}/nodes`)).json.length > before, { label: "template click added node" });
  await finish(ui);
});

test("device rail add-node dialog and link toggle have visible effects", async (t) => {
  const { page, ui } = await workspace(t);
  await button(page, "Add node or image").click();
  await page.getByRole("dialog").waitFor();
  await page.keyboard.press("Escape");
  await button(page, "Add a link").click();
  await page.getByText("Link mode is on").waitFor();
  await button(page, "Add a link").click();
  await page.getByText("Link mode is on").waitFor({ state: "detached" });
  await finish(ui);
});

test("link mode opens a device port picker from a left click", async (t) => {
  const { page, ui } = await workspace(t, { nodes: [{ type: "vpcs", name: "LinkPC" }] });
  await button(page, "Add a link").click();
  await button(page, "Node LinkPC").click();
  await page.getByRole("menuitem", { name: /eth/i }).first().waitFor();
  await finish(ui);
});

test("double-clicking a built-in switch opens its configuration dialog", async (t) => {
  const { page, ui } = await workspace(t, { nodes: [{ type: "ethernet_switch", name: "AccessSwitch" }] });
  await page.getByRole("button", { name: "Node AccessSwitch" }).dblclick();
  const dialog = page.getByRole("dialog", { name: "Configure AccessSwitch" });
  await dialog.waitFor();
  await dialog.getByLabel("Port count").waitFor();
  await dialog.getByLabel("Port VLANs").waitFor();
  await finish(ui);
});

test("device flyout drag adds a node on canvas and over the right dock", async (t) => {
  const { server, page, ui, pid } = await workspace(t);
  const canvas = page.locator(".sim-canvas-surface");
  for (const target of [canvas, page.getByLabel("Workspace docks")]) {
    await button(page, "All devices").click(); const row = page.locator(".sim-device-template").first(); await row.waitFor();
    const before = (await server.api("GET", `/projects/${pid}/nodes`)).json.length;
    await row.dragTo(target, { targetPosition: { x: target === canvas ? 600 : 220, y: 300 } });
    await waitFor(async () => (await server.api("GET", `/projects/${pid}/nodes`)).json.length > before, { label: "drag added node" });
  }
  await finish(ui);
});

test("device flyout drag supports VPCS, switches, and Dynamips templates", async (t) => {
  const { server, page, ui, pid } = await workspace(t);
  const canvas = page.locator(".sim-canvas-surface");
  const templates = [];
  for (const type of ["vpcs", "ethernet_switch", "dynamips"]) {
    const result = await server.api("GET", "/templates");
    const template = result.json.find((item) => item.template_type === type);
    if (!template) t.skip(`no ${type} template is installed`);
    templates.push(template);
  }
  for (const [category, template] of [["End devices", templates[0]], ["Switches", templates[1]], ["Routers", templates[2]]]) {
    await button(page, category).click();
    const row = page.locator(".sim-device-template").filter({ hasText: template.name }).first();
    await row.waitFor();
    const before = (await server.api("GET", `/projects/${pid}/nodes`)).json.length;
    await row.dragTo(canvas, { targetPosition: { x: 500 + before * 30, y: 320 } });
    await waitFor(async () => (await server.api("GET", `/projects/${pid}/nodes`)).json.length === before + 1, { label: `${category} template dropped` });
    await page.getByLabel(`${category} templates`).waitFor();
  }
  await finish(ui);
});

test("canvas wheel zoom is smooth and wheel panning updates the scene", async (t) => {
  const { page, ui } = await workspace(t);
  const svg = page.getByRole("group", { name: "Network topology canvas" });
  const beforeZoom = await zoom(page);
  await page.keyboard.down("Control");
  await page.mouse.move(750, 450);
  await page.mouse.wheel(0, 120);
  await page.keyboard.up("Control");
  await waitFor(async () => (await zoom(page)) < beforeZoom, { label: "wheel zoom" });
  const beforePan = await svg.getAttribute("viewBox");
  await page.mouse.wheel(0, 120);
  await waitFor(async () => (await svg.getAttribute("viewBox")) !== beforePan, { label: "wheel pan" });
  await finish(ui);
});

test("canvas supports middle and space panning plus rubber-band selection", async (t) => {
  const { page, ui } = await workspace(t, { nodes: [{ type: "vpcs", name: "SelectA", x: 0 }, { type: "vpcs", name: "SelectB", x: 250 }] });
  const svg = page.getByRole("group", { name: "Network topology canvas" });
  const canvasBox = await svg.boundingBox();
  const beforeMiddlePan = await svg.getAttribute("viewBox");
  await page.mouse.move(canvasBox.x + 24, canvasBox.y + 24);
  await page.mouse.down({ button: "middle" });
  await page.mouse.move(canvasBox.x + 84, canvasBox.y + 64);
  await page.mouse.up({ button: "middle" });
  await waitFor(async () => (await svg.getAttribute("viewBox")) !== beforeMiddlePan, { label: "middle mouse pan" });

  const beforeSpacePan = await svg.getAttribute("viewBox");
  await page.keyboard.down("Space");
  await page.mouse.move(canvasBox.x + 28, canvasBox.y + 80);
  await page.mouse.down();
  await page.mouse.move(canvasBox.x + 78, canvasBox.y + 110);
  await page.mouse.up();
  await page.keyboard.up("Space");
  await waitFor(async () => (await svg.getAttribute("viewBox")) !== beforeSpacePan, { label: "space drag pan" });

  const node = page.getByRole("button", { name: "Node SelectA" });
  const nodeBox = await node.boundingBox();
  await page.mouse.move(nodeBox.x - 40, nodeBox.y - 40);
  await page.mouse.down();
  await page.mouse.move(nodeBox.x + nodeBox.width / 2, nodeBox.y + nodeBox.height / 2);
  await page.mouse.up();
  await page.locator(".sim-selection-box").waitFor({ state: "detached" });
  await page.locator(".sim-node.is-selected").waitFor();
  await finish(ui);
});

test("right docks reopen after being closed", async (t) => {
  const { page, ui } = await workspace(t);
  const topo = page.getByRole("region", { name: "Topology Summary" }); const servers = page.getByRole("region", { name: "Servers Summary" });
  await button(page, "Collapse Servers Summary").click(); await page.getByLabel("Expand Servers Summary").waitFor();
  await button(page, "Expand Servers Summary").click(); await button(page, "Collapse Servers Summary").waitFor();
  await button(page, "Close Topology Summary").click(); await topo.waitFor({ state: "detached" });
  await button(page, "Close Servers Summary").click(); await servers.waitFor({ state: "detached" });
  await page.getByRole("menuitem", { name: "View" }).click(); await page.getByRole("menuitem", { name: /Topology summary/ }).click(); await topo.waitFor();
  await page.getByRole("menuitem", { name: "View" }).click(); await page.getByRole("menuitem", { name: /Servers summary/ }).click(); await servers.waitFor();
  await page.reload(); await topo.waitFor(); await servers.waitFor();
  await finish(ui);
});

test("topology summary selection selects a node", async (t) => {
  const { page, ui } = await workspace(t, { nodes: [{ type: "vpcs", name: "PC1" }] });
  await page.getByLabel("Topology Summary").getByRole("button", { name: "PC1" }).click();
  assert.ok(await page.getByLabel("Node PC1").evaluate((n) => n.classList.contains("is-selected")));
  await finish(ui);
});

test("status bar counts update, log opens, and resource popover can refresh", async (t) => {
  const { server, page, ui, pid } = await workspace(t);
  const status = page.locator(".sim-statusbar");
  await status.getByText("0 devices", { exact: true }).waitFor();
  await addNode(server, pid, await templateByType(server, "vpcs"), 20, 20, "StatusPC");
  await page.getByLabel("Node StatusPC").waitFor(); await status.getByText("1 devices", { exact: true }).waitFor();
  await button(page, /Open project log/).click(); await page.getByRole("dialog", { name: "Project log console" }).waitFor(); await page.keyboard.press("Escape");
  await page.locator(".sim-resource-trigger").click(); await page.getByText("Resource monitor", { exact: true }).waitFor();
  await button(page, "Refresh resource monitor").click(); await finish(ui);
});

test("keyboard shortcuts select all, clear selection, delete, escape link mode, and move with arrows", async (t) => {
  const { server, page, ui, pid } = await workspace(t, { nodes: [{ type: "vpcs", name: "KeyPC" }] });
  const canvas = page.getByRole("group", { name: "Network topology canvas" }); await canvas.focus(); await page.keyboard.press("Control+a");
  await page.locator(".sim-node.is-selected").waitFor();
  const before = (await server.api("GET", `/projects/${pid}/nodes`)).json[0];
  await page.keyboard.press("ArrowRight");
  await waitFor(async () => { const n = (await server.api("GET", `/projects/${pid}/nodes`)).json[0]; return n.x !== before.x || n.y !== before.y; }, { label: "arrow moved node" });
  await page.keyboard.press("Escape"); await page.locator(".sim-node.is-selected").waitFor({ state: "detached" });
  await button(page, "Add a link").click(); await canvas.focus(); await page.keyboard.press("Escape"); await page.getByText("Link mode is on").waitFor({ state: "detached" });
  await canvas.focus(); await page.keyboard.press("Control+a");
  const dialogPromise = page.waitForEvent("dialog");
  await page.keyboard.press("Delete");
  const dialog = await dialogPromise;
  await dialog.accept();
  await waitFor(async () => (await server.api("GET", `/projects/${pid}/nodes`)).json.length === 0, { label: "Delete shortcut removed node" });
  await finish(ui);
});

test("floating console opens, moves, resizes, minimizes, restores, closes, and persists state", async (t) => {
  const { server, page, ui, pid } = await workspace(t, { nodes: [{ type: "vpcs", name: "ConsolePC" }] });
  await button(page, "Node ConsolePC").click(); await button(page, "Start all").click();
  await waitFor(async () => (await server.api("GET", `/projects/${pid}/nodes`)).json[0]?.status === "started", { label: "console target started" });
  await page.getByRole("button", { name: /Open console/ }).click();
  const dock = page.getByRole("region", { name: "Device consoles" }); await dock.waitFor();
  const before = await dock.boundingBox(); const title = page.getByRole("tablist", { name: "Open consoles" });
  const titleBox = await title.boundingBox();
  await page.mouse.move(titleBox.x + 40, titleBox.y + 16); await page.mouse.down();
  await page.mouse.move(titleBox.x + 150, titleBox.y + 96, { steps: 6 }); await page.mouse.up();
  const moved = await dock.boundingBox(); assert.ok(moved.x !== before.x || moved.y !== before.y);
  const resize = page.locator(".sim-console-resize"); const rb = await resize.boundingBox(); await page.mouse.move(rb.x + 5, rb.y + 5); await page.mouse.down(); await page.mouse.move(rb.x + 50, rb.y + 40, { steps: 3 }); await page.mouse.up();
  const resized = await dock.boundingBox();
  await button(page, "Minimize console dock").click(); await button(page, "Restore console dock").waitFor();
  await button(page, "Restore console dock").click(); await button(page, "Close console dock").click(); await dock.waitFor({ state: "detached" });
  const stored = await page.evaluate(() => JSON.parse(localStorage.getItem("ns.console.float")));
  assert.equal(stored.open, false); assert.ok(stored.width > 0 && stored.height > 0);
  await finish(ui);
  assert.ok(resized.width > moved.width || resized.height > moved.height, "resize handle changes panel dimensions");
});
