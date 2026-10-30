import test from "node:test";
import assert from "node:assert/strict";
import { accessSync, constants, copyFileSync, existsSync, mkdtempSync, readdirSync, rmSync } from "node:fs";
import { join } from "node:path";
import { startServer, newProject, launchPage, openProject, waitFor } from "./harness.mjs";

const imageDir = join(process.env.HOME, ".local/share/network-simulator/images/dynamips");
const qemuImageDir = join(process.env.HOME, ".local/share/network-simulator/images/qemu");
function executableOnPath(name) {
  const candidates = [join(process.env.HOME, ".local/share/network-simulator/bin", name), ...String(process.env.PATH ?? "").split(":").map((dir) => join(dir, name))];
  return candidates.find((path) => {
    try { accessSync(path, constants.X_OK); return true; } catch { return false; }
  });
}
function imageFor(platform) {
  try { return readdirSync(imageDir).find((file) => file.toLowerCase().includes(platform) && /\.image$|\.bin$/i.test(file)); } catch { return undefined; }
}
const dynamipsBinary = executableOnPath("dynamips");
const dynamipsImages = Object.fromEntries(["c3620", "c7200"].map((platform) => [platform, imageFor(platform)]));
const dynamipsSkipReason = !dynamipsBinary
  ? "Dynamips binary is absent from PATH and ~/.local/share/network-simulator/bin"
  : !Object.values(dynamipsImages).some(Boolean)
    ? `no c3620/c7200 IOS image found in ${imageDir}`
    : undefined;

async function setup(t, name) {
  const server = await startServer();
  const ui = await launchPage(server);
  const pid = await newProject(server, name);
  t.after(async () => {
    assert.deepEqual(ui.errors, []);
    try {
      for (const node of await nodes(server, pid)) {
        if (node.status !== "stopped") await server.api("POST", `/projects/${pid}/nodes/${node.node_id}/stop`);
      }
    } catch {}
    await ui.close();
    const log = server.log();
    await server.stop();
    assert.doesNotMatch(log, /database is locked/i);
    assert.doesNotMatch(log, /\b500\b/);
  });
  return { server, ui, pid };
}

async function templates(server) {
  const result = await server.api("GET", "/templates");
  assert.equal(result.status, 200);
  return result.json;
}

async function dragTemplate(page, template, nodeName) {
  const category = template.template_type === "ethernet_switch" || template.template_type === "ethernet_hub" ? "Switches" : template.template_type === "dynamips" || /cisco|router|dynamips/i.test(template.name) ? "Routers" : "End devices";
  await page.locator(`button[aria-label="${category}"]`).click();
  const card = page.locator(".sim-device-template").filter({ hasText: template.name }).first();
  await card.waitFor();
  const oldCount = await page.locator(".sim-node").count();
  const canvas = page.locator(".sim-canvas-surface");
  const box = await canvas.boundingBox();
  assert.ok(box);
  const fraction = oldCount === 0 ? 0.38 : oldCount === 1 ? 0.62 : 0.48;
  await card.dragTo(canvas, { targetPosition: { x: box.width * fraction, y: box.height * 0.45 }, steps: 12 });
  await waitFor(async () => await page.locator(".sim-node").count() > oldCount, { timeout: 15000, label: `dragged ${nodeName}` });
  return page.locator(".sim-node").nth(oldCount);
}

async function nodeName(locator) { return (await locator.getAttribute("aria-label")).replace(/^Node /, ""); }

async function nodes(server, pid) {
  const result = await server.api("GET", `/projects/${pid}/nodes`);
  assert.equal(result.status, 200);
  return result.json;
}

async function statusIs(server, pid, name, status, timeout = 15000) {
  return waitFor(async () => (await nodes(server, pid)).find((node) => node.name === name)?.status === status,
    { timeout, label: `${name} ${status}` });
}

async function clickMenu(page, menu, item) {
  await page.getByRole("menuitem", { name: item, exact: true }).click();
}

test("VPCS lifecycle paths work from toolbar, Control, and context menu; suspend is unavailable", async (t) => {
  const { server, ui, pid } = await setup(t, "device-lifecycle-vpcs");
  const vpcs = (await templates(server)).find((item) => item.template_type === "vpcs");
  assert.ok(vpcs);
  await openProject(server, ui.page, pid);
  const node = await dragTemplate(ui.page, vpcs, "PC1");
  const pcName = await nodeName(node);
  const dot = node.locator(".sim-node-status");

  await ui.page.getByRole("button", { name: "Start all" }).first().click();
  await statusIs(server, pid, pcName, "started");
  await node.waitFor();
  await node.locator(".sim-node-status.running").waitFor({ timeout: 15000 });
  assert.equal(await ui.page.getByRole("button", { name: "Suspend all" }).isDisabled(), true);
  await ui.page.getByRole("button", { name: "Start all" }).first().click();
  await statusIs(server, pid, pcName, "started");
  await ui.page.getByRole("button", { name: "Stop all" }).click();
  await statusIs(server, pid, pcName, "stopped");
  await ui.page.getByRole("button", { name: "Start all" }).first().click();
  await statusIs(server, pid, pcName, "started");
  await ui.page.getByRole("button", { name: "Reload all" }).click();
  await statusIs(server, pid, pcName, "started");
  await ui.page.reload();
  await ui.page.locator(`[aria-label="Node ${pcName}"]`).waitFor();
  assert.equal((await nodes(server, pid))[0].status, "started");
  await ui.page.getByRole("button", { name: "Stop all" }).click();
  await statusIs(server, pid, pcName, "stopped");

  await ui.page.getByRole("menuitem", { name: "Control" }).click();
  await clickMenu(ui.page, "Control", "Start all");
  await statusIs(server, pid, pcName, "started");
  await ui.page.getByRole("menuitem", { name: "Control" }).click();
  assert.equal(await ui.page.getByRole("menuitem", { name: "Suspend all", exact: true }).isDisabled(), true);
  await ui.page.keyboard.press("Escape");
  await ui.page.getByRole("menuitem", { name: "Control" }).click();
  await clickMenu(ui.page, "Control", "Stop all");
  await statusIs(server, pid, pcName, "stopped");

  await node.click({ button: "right" });
  assert.equal(await ui.page.getByRole("menuitem", { name: "Suspend device", exact: true }).count(), 0);
  await ui.page.getByRole("menuitem", { name: "Start device" }).click();
  await statusIs(server, pid, pcName, "started");
  await node.click({ button: "right" });
  await ui.page.getByRole("menuitem", { name: "Stop device", exact: true }).click();
  await statusIs(server, pid, pcName, "stopped");
  assert.ok(dot);
  assert.equal((await nodes(server, pid))[0].status, "stopped");
});

test("VPCS console accepts commands and reports interface configuration", async (t) => {
  const { server, ui, pid } = await setup(t, "device-console-vpcs");
  const vpcs = (await templates(server)).find((item) => item.template_type === "vpcs");
  assert.ok(vpcs);
  await openProject(server, ui.page, pid);
  const node = await dragTemplate(ui.page, vpcs, "ConsolePC");
  const name = await nodeName(node);
  await ui.page.getByRole("button", { name: "Start all" }).first().click();
  await statusIs(server, pid, name, "started");
  await node.click({ button: "right" });
  await ui.page.getByRole("menuitem", { name: "Open console" }).click();
  const consolePanel = ui.page.locator('[aria-label*="Console"]');
  await consolePanel.first().waitFor();
  const input = ui.page.locator("textarea").last();
  await input.fill("ip 10.0.0.1/24\n");
  await waitFor(async () => /10\.0\.0\.1|IP\/MASK/i.test(await ui.page.locator("body").innerText()), { label: "VPCS IP response" });
});

test("device context actions rename, duplicate, lock, change symbol, isolate, configure and delete", async (t) => {
  const { server, ui, pid } = await setup(t, "device-actions");
  const all = await templates(server);
  const vpcs = all.find((item) => item.template_type === "vpcs");
  assert.ok(vpcs);
  await openProject(server, ui.page, pid);
  const node = await dragTemplate(ui.page, vpcs, "ActionsPC");
  const before = (await nodes(server, pid))[0];
  await node.click({ button: "right" });
  await ui.page.getByRole("menuitem", { name: "Edit device name" }).click();
  await ui.page.getByRole("button", { name: "Rename device" }).click();
  const nameInput = ui.page.locator("[data-rename-node]");
  await nameInput.waitFor();
  await nameInput.fill("RenamedPC");
  await ui.page.getByRole("button", { name: "Save device name" }).click();
  await waitFor(async () => (await nodes(server, pid)).some((item) => item.name === "RenamedPC"), { label: "renamed node" });
  await ui.page.locator('[aria-label="Node RenamedPC"]').click({ button: "right" });
  await ui.page.getByRole("menuitem", { name: "Duplicate device" }).click();
  await waitFor(async () => (await nodes(server, pid)).length === 2, { label: "duplicate node" });
  await ui.page.locator('[aria-label="Node RenamedPC"]').click({ button: "right" });
  await ui.page.getByRole("menuitem", { name: "Lock device" }).click();
  await waitFor(async () => (await nodes(server, pid)).find((item) => item.node_id === before.node_id)?.locked, { label: "locked node" });
  await ui.page.locator('[aria-label="Node RenamedPC"]').click({ button: "right" });
  await ui.page.getByRole("menuitem", { name: "Isolate device" }).click();
  await waitFor(async () => (await nodes(server, pid)).find((item) => item.node_id === before.node_id)?.isolated, { label: "isolated node" });
  await ui.page.locator('[aria-label="Node RenamedPC"]').click({ button: "right" });
  await ui.page.getByRole("menuitem", { name: "Change symbol" }).click();
  await ui.page.getByRole("dialog").waitFor();
  await ui.page.keyboard.press("Escape");
  await ui.page.locator('[aria-label="Node RenamedPC"]').click({ button: "right" });
  await ui.page.getByRole("menuitem", { name: "Unlock device" }).click();
  await waitFor(async () => !(await nodes(server, pid)).find((item) => item.node_id === before.node_id)?.locked, { label: "unlocked node" });
  await ui.page.keyboard.press("Escape");
  const duplicate = (await nodes(server, pid)).find((item) => item.node_id !== before.node_id);
  assert.ok(duplicate);
  await ui.page.locator(`[aria-label="Node ${duplicate.name}"]`).click({ button: "right" });
  await ui.page.getByRole("menuitem", { name: "Delete device" }).click();
  await waitFor(async () => !(await nodes(server, pid)).some((item) => item.node_id === duplicate.node_id), { label: "deleted duplicate node" });
});

test("drag from rail creates VPCS and switch devices; started VPCS to switch link can be deleted", async (t) => {
  const { server, ui, pid } = await setup(t, "device-links");
  const all = await templates(server);
  const pc = all.find((item) => item.template_type === "vpcs");
  const sw = all.find((item) => item.template_type === "ethernet_switch");
  assert.ok(pc && sw);
  await openProject(server, ui.page, pid);
  const first = await dragTemplate(ui.page, pc, "LinkPC");
  const second = await dragTemplate(ui.page, sw, "LinkSwitch");
  const firstName = await nodeName(first);
  const secondName = await nodeName(second);
  await ui.page.getByRole("button", { name: "Start all" }).first().click();
  await statusIs(server, pid, firstName, "started");
  assert.equal((await nodes(server, pid)).find((item) => item.name === secondName)?.status, "started");
  await ui.page.getByRole("button", { name: "Draw link" }).click();
  await first.click({ button: "right" });
  const portMenu = ui.page.getByRole("menuitem", { name: /^eth\d+$/ });
  await portMenu.first().waitFor();
  await ui.page.waitForTimeout(400);
  await portMenu.first().click();
  await ui.page.waitForTimeout(500);
  await second.click({ button: "right" });
  const switchPorts = ui.page.getByRole("menuitem", { name: /^Ethernet\d+$/ });
  await ui.page.getByRole("menuitem", { name: "Ethernet7", exact: true }).waitFor();
  await switchPorts.first().waitFor();
  await ui.page.waitForTimeout(400);
  await switchPorts.first().click();
  await ui.page.waitForTimeout(500);
  try {
    await waitFor(async () => {
      const result = await server.api("GET", `/projects/${pid}/links`);
      return result.status === 200 && result.json.length === 1;
    }, { timeout: 15000, label: "link created" });
  } catch (error) {
    throw error;
  }
  const link = ui.page.getByRole("button", { name: /Ethernet segment connecting/ });
  await link.click({ button: "right" });
  await ui.page.getByRole("menuitem", { name: "Delete link" }).click();
  await waitFor(async () => (await server.api("GET", `/projects/${pid}/links`)).json.length === 0, { label: "link deleted" });
});

test("NAT and cloud devices can be added from the device flyout", async (t) => {
  const { server, ui, pid } = await setup(t, "device-network-nodes");
  const all = await templates(server);
  const choices = [all.find((item) => item.template_type === "nat"), all.find((item) => item.template_type === "cloud")].filter(Boolean);
  assert.equal(choices.length, 2, "built in NAT and cloud templates are available");
  await openProject(server, ui.page, pid);
  for (const [index, template] of choices.entries()) {
    const node = await dragTemplate(ui.page, template, `NetworkNode${index}`);
    const name = await nodeName(node);
    assert.ok((await nodes(server, pid)).some((item) => item.name === name && item.node_type === template.template_type));
  }
});

test("starting a QEMU node with a missing image shows a visible error", async (t) => {
  const { server, ui, pid } = await setup(t, "device-missing-runtime");
  const image = join(qemuImageDir, "mikrotik-chr-7.23.8.qcow2");
  assert.ok(existsSync(image), "local QEMU image is available to seed the node");
  const tempImageDir = mkdtempSync(join(qemuImageDir, "e2e-missing-runtime-"));
  const tempImage = join(tempImageDir, "seed.qcow2");
  copyFileSync(image, tempImage);
  t.after(() => rmSync(tempImageDir, { recursive: true, force: true }));
  const created = await server.api("POST", "/templates", {
    name: "Missing image QEMU", template_type: "qemu", hda_disk_image: tempImage,
    adapters: 1, memory: 512, cpus: 1, platform: "x86_64", adapter_type: "e1000",
    hda_disk_interface: "ide", boot_priority: "c", linked_clone: false,
  });
  assert.equal(created.status, 201, JSON.stringify(created));
  const add = await server.api("POST", `/projects/${pid}/templates/${created.json.template_id}`, { x: 0, y: 0, compute_id: "local" });
  assert.equal(add.status, 201, JSON.stringify(add));
  rmSync(tempImageDir, { recursive: true, force: true });
  await openProject(server, ui.page, pid);
  const node = ui.page.locator(`[aria-label="Node ${add.json.name}"]`);
  await node.waitFor();
  await node.click({ button: "right" });
  await ui.page.getByRole("menuitem", { name: "Start device", exact: true }).click();
  try {
    await ui.page.getByRole("alert").waitFor();
  } catch (error) {
    console.error("missing runtime diagnostics", (await ui.page.locator("body").innerText()).slice(-500), server.log().slice(-1000), await nodes(server, pid));
    throw error;
  }
  assert.ok((await ui.page.getByRole("alert").innerText()).trim(), "the runtime failure toast has a visible message");
});

async function dragNodeBy(page, locator, dx, dy) {
  const box = await locator.boundingBox();
  assert.ok(box, "node has a visible canvas position");
  const x = box.x + box.width / 2;
  const y = box.y + box.height * 0.3;
  await page.mouse.move(x, y);
  await page.mouse.down();
  await page.mouse.move(x + dx, y + dy, { steps: 8 });
  await page.mouse.up();
}

test("Ctrl multi-select and rubber-band selection move node groups; locked nodes stay put", async (t) => {
  const { server, ui, pid } = await setup(t, "device-selection-drag");
  const pc = (await templates(server)).find((item) => item.template_type === "vpcs");
  assert.ok(pc);
  await openProject(server, ui.page, pid);
  const first = await dragTemplate(ui.page, pc, "GroupA");
  const second = await dragTemplate(ui.page, pc, "GroupB");
  const names = [await nodeName(first), await nodeName(second)];

  await first.click({ modifiers: ["Control"] });
  await second.click({ modifiers: ["Control"] });
  await waitFor(async () => await ui.page.locator(".sim-node.is-selected").count() === 2, { label: "Ctrl multi-selection" });
  const boxes = await Promise.all(names.map((name) => ui.page.locator(`[aria-label="Node ${name}"]`).boundingBox()));
  assert.ok(boxes.every(Boolean));
  const left = Math.min(...boxes.map((box) => box.x)) - 24;
  const top = Math.min(...boxes.map((box) => box.y)) - 24;
  const right = Math.max(...boxes.map((box) => box.x + box.width)) + 24;
  const bottom = Math.max(...boxes.map((box) => box.y + box.height)) + 24;
  await ui.page.mouse.move(left, top);
  await ui.page.mouse.down();
  await ui.page.mouse.move(right, bottom, { steps: 8 });
  await ui.page.mouse.up();
  await waitFor(async () => await ui.page.locator(".sim-node.is-selected").count() === 2, { label: "rubber-band multi-selection" });
  const initial = await nodes(server, pid);
  await dragNodeBy(ui.page, ui.page.locator(`[aria-label="Node ${names[0]}"]`), 70, 40);
  await waitFor(async () => (await nodes(server, pid)).some((node) => {
    const old = initial.find((item) => item.node_id === node.node_id);
    return Number(node.x) !== Number(old?.x) || Number(node.y) !== Number(old?.y);
  }), { label: "rubber-band selected nodes moved" });
  const moved = await nodes(server, pid);
  const deltas = names.map((name) => {
    const old = initial.find((item) => item.name === name);
    const current = moved.find((item) => item.name === name);
    return { x: Number(current.x) - Number(old.x), y: Number(current.y) - Number(old.y) };
  });
  assert.deepEqual(deltas[1], deltas[0], "rubber-band selected nodes move by the same REST position delta");

  const locked = ui.page.locator(`[aria-label="Node ${names[0]}"]`);
  await locked.click({ button: "right" });
  await ui.page.getByRole("menuitem", { name: "Lock device" }).click();
  await waitFor(async () => (await nodes(server, pid)).find((item) => item.name === names[0])?.locked, { label: "node locked" });
  const lockedBefore = (await nodes(server, pid)).find((item) => item.name === names[0]);
  await dragNodeBy(ui.page, locked, 100, 80);
  await new Promise((resolve) => setTimeout(resolve, 500));
  const lockedAfter = (await nodes(server, pid)).find((item) => item.name === names[0]);
  assert.deepEqual({ x: lockedAfter.x, y: lockedAfter.y }, { x: lockedBefore.x, y: lockedBefore.y });
});

test("Ethernet switch and hub lifecycle remains started and controls are unavailable", async (t) => {
  const { server, ui, pid } = await setup(t, "device-switch-hub");
  const all = await templates(server);
  const choices = [
    all.find((item) => item.template_type === "ethernet_switch"),
    all.find((item) => item.template_type === "ethernet_hub"),
  ].filter(Boolean);
  assert.ok(choices.length > 0, "at least one built in switching device is available");
  await openProject(server, ui.page, pid);
  for (const [index, template] of choices.entries()) {
    const name = `Switching${index}`;
    const node = await dragTemplate(ui.page, template, name);
    await node.click({ button: "right" });
    const actualName = await nodeName(node);
    assert.ok((await nodes(server, pid)).some((item) => item.name === actualName));
    assert.equal(await ui.page.getByRole("menuitem", { name: "Start device", exact: true }).count(), 0);
    await ui.page.keyboard.press("Escape");
    assert.equal((await nodes(server, pid)).find((item) => item.name === actualName)?.status, "started");
    await new Promise((resolve) => setTimeout(resolve, 1000));
    assert.equal((await nodes(server, pid)).find((item) => item.name === actualName)?.status, "started");
  }
});

test("Dynamips Cisco routers start, link, suspend, resume, reload, stop, and produce console output", { skip: dynamipsSkipReason }, async (t) => {
  const { server, ui, pid } = await setup(t, "device-dynamips");
  const all = await templates(server);
  const dynamips = all.filter((item) => item.template_type === "dynamips" && /c3620|c7200/i.test(`${item.name} ${item.platform} ${item.image}`));
  for (const [index, platform] of ["c3620", "c7200"].entries()) {
    if (!dynamipsImages[platform]) { t.diagnostic(`${platform} coverage skipped: no matching IOS image in ${imageDir}`); continue; }
    const template = dynamips.find((item) => `${item.name} ${item.platform} ${item.image}`.toLowerCase().includes(platform));
    assert.ok(template, `${platform} IOS image exists but no matching Dynamips template is exposed`);
    await openProject(server, ui.page, pid);
    const node = await dragTemplate(ui.page, template, `Router${index}`);
    const name = await nodeName(node);
    await node.click({ button: "right" });
    await ui.page.getByRole("menuitem", { name: "Start device" }).click();
    await statusIs(server, pid, name, "started", 60000);
    await new Promise((resolve) => setTimeout(resolve, 5000));
    assert.equal((await nodes(server, pid)).find((item) => item.name === name)?.status, "started");
    const consoleFrames = [];
    ui.page.on("websocket", (socket) => socket.on("framereceived", (frame) => {
      const value = typeof frame.payload === "string" ? frame.payload : Buffer.from(frame.payload).toString("latin1");
      consoleFrames.push(value);
    }));
    await node.click({ button: "right" });
    await ui.page.getByRole("menuitem", { name: "Open console" }).click();
    const consolePanel = ui.page.getByRole("region", { name: `Console for ${name}` });
    await consolePanel.waitFor();
    await consolePanel.locator(".sim-xterm").waitFor();
    await consolePanel.locator(".sim-xterm").click();
    await ui.page.keyboard.press("Enter");
    await waitFor(async () => consoleFrames.some((frame) => /Cisco|Press RETURN|Router|rommon|boot/i.test(frame)), { timeout: 60000, label: `${platform} console output` });

    const swTemplate = all.find((item) => item.template_type === "ethernet_switch");
    assert.ok(swTemplate, "built in Ethernet switch template is available");
    const switchNode = await dragTemplate(ui.page, swTemplate, `RouterSwitch${index}`);
    await ui.page.getByRole("button", { name: "Draw link" }).click();
    await node.click({ button: "right" });
    const ports = ui.page.getByRole("menuitem", { name: /^(eth|Ethernet)\d+$/ });
    await ports.first().waitFor();
    await ports.first().click();
    await switchNode.click({ button: "right" });
    await ports.first().waitFor();
    await ports.first().click();
    await waitFor(async () => (await server.api("GET", `/projects/${pid}/links`)).json.length === 1, { label: "router to switch link" });
    const link = ui.page.getByRole("button", { name: /Ethernet segment connecting/ });
    await link.click({ button: "right" });
    await ui.page.getByRole("menuitem", { name: "Delete link" }).click();
    await waitFor(async () => (await server.api("GET", `/projects/${pid}/links`)).json.length === 0, { label: "router switch link removal" });

    await node.click({ button: "right" });
    const suspend = ui.page.getByRole("menuitem", { name: "Suspend device", exact: true });
    if (await suspend.count()) {
      await suspend.click();
      await statusIs(server, pid, name, "suspended", 60000);
      await node.click({ button: "right" });
      await ui.page.getByRole("menuitem", { name: "Resume device", exact: true }).click();
      await statusIs(server, pid, name, "started", 60000);
    }
    await node.click({ button: "right" });
    const reload = ui.page.getByRole("menuitem", { name: "Reload device", exact: true });
    if (await reload.count()) {
      await reload.click();
      await statusIs(server, pid, name, "started", 60000);
    }
    await node.click({ button: "right" });
    await ui.page.getByRole("menuitem", { name: "Stop device", exact: true }).click();
    await statusIs(server, pid, name, "stopped", 60000);
  }
});
