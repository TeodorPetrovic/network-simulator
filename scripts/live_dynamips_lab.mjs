#!/usr/bin/env node

// Opt-in live IOS integration scenario. This intentionally uses the controller's
// v3 routes and globally installed image library. It never writes image data.

import assert from "node:assert/strict";
import fs from "node:fs";
import os from "node:os";
import path from "node:path";
import { setTimeout as sleep } from "node:timers/promises";
import { execFileSync } from "node:child_process";

const base = (process.env.NETWORK_SIMULATOR_BASE_URL ?? "http://127.0.0.1:3082").replace(/\/$/, "");
const results = [];
let projectId;
const consoles = new Set();
const runtimeDirectories = new Set();
const dynamipsImageDir = process.env.NETWORK_SIMULATOR_DYNAMIPS_IMAGES
  ?? path.join(process.env.NETWORK_SIMULATOR_IMAGE_HOME
    ?? path.join(os.homedir(), ".local/share/network-simulator/images"), "dynamips");

const familyCases = [
  { family: "c1700", platform: "c1700", chassis: "1720", prefixes: ["c1700-", "ishare-1-c1710-"], ram: 160, nvram: 128, slot0: "C1700-MB-1FE", idlepc: "0x80358a60", iomem: 15, sparsemem: false },
  { family: "c2600", platform: "c2600", chassis: "2610", prefixes: ["c2600-", "ishare-3-c2600-"], ram: 160, nvram: 128, slot0: "C2600-MB-1E", idlepc: "0x8027ec88", iomem: 15, sparsemem: false },
  { family: "c2691", platform: "c2691", prefixes: ["c2691-", "ishare-6-c2691-"], ram: 192, nvram: 256, slot0: "GT96100-FE", idlepc: "0x60bcf9f8", iomem: 5, sparsemem: true },
  { family: "c3620", platform: "c3600", chassis: "3620", prefixes: ["c3620-"], ram: 192, nvram: 256, slot0: "NM-1FE-TX", idlepc: "0x603a8bac", iomem: 5, sparsemem: true },
  { family: "c3640", platform: "c3600", chassis: "3640", prefixes: ["c3640-", "ishare-8-c3640-"], ram: 192, nvram: 256, slot0: "NM-1FE-TX", idlepc: "0x6050b114", iomem: 5, sparsemem: true },
  { family: "c3660", platform: "c3600", chassis: "3660", prefixes: ["c3660-", "ishare-10-c3660-"], ram: 192, nvram: 256, slot0: "Leopard-2FE", idlepc: "0x6076e0b4", iomem: 5, sparsemem: true },
  { family: "c3725", platform: "c3725", prefixes: ["c3725-", "ishare-16-c3725-"], ram: 128, nvram: 256, slot0: "GT96100-FE", idlepc: "0x60c09aa0", iomem: 5, sparsemem: true },
  { family: "c3745", platform: "c3745", prefixes: ["c3745-", "ishare-17-c3745-"], ram: 256, nvram: 256, slot0: "GT96100-FE", slot1: "NM-1FE-TX", slot2: "NM-4T", wic0: "WIC-1T", wic1: "WIC-1T", wic2: "WIC-1T", idlepc: "0x60aa1da0", iomem: 5, sparsemem: true },
  { family: "c7200", platform: "c7200", prefixes: ["c7200-", "ishare-22-c7200-"], ram: 512, nvram: 512, slot0: "C7200-IO-FE", npe: "npe-400", midplane: "vxr", idlepc: "0x6066a998", sparsemem: true },
];
const requestedPlatforms = (process.env.NETWORK_SIMULATOR_TEST_PLATFORMS ?? "")
  .split(",").map((family) => family.trim().toLowerCase()).filter(Boolean);
const selectedFamilyCases = requestedPlatforms.length
  ? familyCases.filter((config) => requestedPlatforms.includes(config.family))
  : familyCases;
assert.ok(selectedFamilyCases.length > 0, `NETWORK_SIMULATOR_TEST_PLATFORMS selected no known family: ${requestedPlatforms.join(",")}`);

async function api(path, { method = "GET", body } = {}) {
  const response = await fetch(`${base}${path}`, {
    method,
    headers: body === undefined ? undefined : { "content-type": "application/json" },
    body: body === undefined ? undefined : JSON.stringify(body),
  });
  const text = await response.text();
  let value;
  try { value = text ? JSON.parse(text) : undefined; } catch { value = text; }
  if (!response.ok) throw new Error(`${method} ${path}: HTTP ${response.status}: ${JSON.stringify(value)}`);
  return value;
}

function record(name, detail) {
  results.push({ name, detail });
  console.log(`PASS ${name}: ${detail}`);
}

function openConsole(path) {
  const url = `${base.replace(/^http/, "ws")}${path}`;
  const socket = new WebSocket(url);
  consoles.add(socket);
  let buffer = "";
  let closed = false;
  const opened = new Promise((resolve, reject) => {
    socket.addEventListener("open", resolve, { once: true });
    socket.addEventListener("error", () => reject(new Error(`WebSocket failed: ${url}`)), { once: true });
  });
  socket.addEventListener("message", async (event) => {
    let text = event.data;
    if (typeof text !== "string") {
      if (text instanceof ArrayBuffer) text = Buffer.from(text).toString("utf8");
      else if (ArrayBuffer.isView(text)) text = Buffer.from(text.buffer, text.byteOffset, text.byteLength).toString("utf8");
      else if (text && typeof text.text === "function") text = await text.text();
      else text = String(text);
    }
    buffer += text;
    if (buffer.length > 256_000) buffer = buffer.slice(-128_000);
  });
  socket.addEventListener("close", () => { closed = true; });
  return {
    socket,
    async ready() { await opened; },
    text() { return buffer.replace(/\x1b\[[0-9;?]*[ -/]*[@-~]/g, ""); },
    isClosed() { return closed; },
    send(text) { socket.send(text); },
    async wait(regex, timeoutMs = 20_000, label = regex.toString()) {
      const deadline = Date.now() + timeoutMs;
      while (Date.now() < deadline) {
        const content = this.text();
        if (closed) throw new Error(`Console closed while waiting for ${label}; last output: ${content.slice(-2500)}`);
        const match = content.match(regex);
        if (match) return { content, match };
        await sleep(100);
      }
      throw new Error(`Console timed out waiting for ${label}; last output: ${this.text().slice(-2500)}`);
    },
    close() { try { socket.close(); } catch {} consoles.delete(socket); },
  };
}

async function bootIos(project, node) {
  const console = openConsole(`/v3/compute/projects/${project}/dynamips/nodes/${node.node_id}/console/ws`);
  await console.ready();
  const deadline = Date.now() + 180_000;
  let answeredDialog = false;
  let pressedReturn = false;
  let pressedConsoleEnter = false;
  while (Date.now() < deadline) {
    const output = console.text();
    if (console.isClosed()) {
      const status = await api(`/v3/projects/${project}/nodes/${node.node_id}`).catch((error) => ({ error: error.message }));
      let engine = "";
      if (node.runtime_working_dir) {
        const path = `${node.runtime_working_dir}/dynamips_engine.log`;
        if (fs.existsSync(path)) engine = fs.readFileSync(path, "utf8").slice(-10000);
      }
      const processRows = node.runtime_hypervisor_port ? matchingDynamips(node.runtime_hypervisor_port) : [];
      throw new Error(`Dynamips console closed during IOS boot for ${node.name}; node=${JSON.stringify(status)}; hypervisor=${JSON.stringify(processRows)}; engine log=${engine}; console tail: ${output.slice(-5000)}`);
    }
    if (!answeredDialog && /initial configuration dialog[\s\S]{0,180}\[yes\/no\]/i.test(output)) {
      console.send("no\r");
      answeredDialog = true;
    }
    if (!pressedReturn && /press\s+return\s+to\s+get\s+started/i.test(output)) {
      console.send("\r");
      pressedReturn = true;
    }
    if (!pressedConsoleEnter && /press\s+enter\s+to\s+get\s+the\s+prompt/i.test(output)) {
      console.send("\r");
      pressedConsoleEnter = true;
    }
    if (/\b[A-Za-z0-9_.-]+[>#]/.test(output)) {
      record(`IOS boot ${node.name}`, `console reached IOS exec prompt${answeredDialog ? " after declining initial configuration" : ""}`);
      return console;
    }
    await sleep(150);
  }
  throw new Error(`IOS boot timed out for ${node.name}; console tail: ${console.text().slice(-5000)}`);
}

async function iosCommand(console, command, prompt = /[>#]\s*$/m, timeoutMs = 15_000) {
  const start = console.text().length;
  console.send(`${command}\r`);
  const deadline = Date.now() + timeoutMs;
  while (Date.now() < deadline) {
    const output = console.text().slice(start);
    if (prompt.test(output)) return output;
    await sleep(80);
  }
  throw new Error(`IOS command timed out: ${command}; output: ${console.text().slice(start).slice(-1200)}`);
}

async function iosPing(console, address) {
  let output = "";
  for (let attempt = 0; attempt < 3; attempt += 1) {
    output = await iosCommand(console, `ping ${address}`, /#\s*$/m, 30_000);
    if (/Success rate is\s+100 percent/i.test(output)) return output;
    await sleep(400);
  }
  return output;
}

async function setStartupAddress(project, node, ip, mask = "255.255.255.0") {
  const startupConfig = [
    `hostname ${node.name}`,
    "no ip domain-lookup",
    "interface FastEthernet0/0",
    `ip address ${ip} ${mask}`,
    "no shutdown",
    "end",
    "",
  ].join("\n");
  await api(`/v3/projects/${project}/nodes/${node.node_id}`, {
    method: "PUT", body: { startup_config: startupConfig },
  });
}

async function vpcsConsole(project, node) {
  const console = openConsole(`/v3/compute/projects/${project}/vpcs/nodes/${node.node_id}/console/ws`);
  await console.ready();
  await console.wait(/VPCS>/, 10_000, "VPCS prompt");
  return console;
}

async function vpcsCommand(console, command, timeoutMs = 10_000) {
  const start = console.text().length;
  console.send(`${command}\r`);
  const deadline = Date.now() + timeoutMs;
  while (Date.now() < deadline) {
    const output = console.text().slice(start);
    if (/VPCS>\s*$/m.test(output)) return output;
    if (console.isClosed()) throw new Error(`VPCS console closed during '${command}'`);
    await sleep(80);
  }
  throw new Error(`VPCS command timed out: ${command}; output: ${console.text().slice(start).slice(-1200)}`);
}

async function pingVpcs(console, address) {
  let output = "";
  for (let attempt = 0; attempt < 3; attempt += 1) {
    output = await vpcsCommand(console, `ping ${address}`, 15_000);
    if (/(?:bytes from|icmp_seq=|reply from)/i.test(output)) return output;
    await sleep(400);
  }
  return output;
}

function imageForFamily(config) {
  const fileName = fs.readdirSync(dynamipsImageDir).find((candidate) =>
    candidate.endsWith(".image") && config.prefixes.some((prefix) => candidate.startsWith(prefix)));
  assert.ok(fileName, `no IOS image for ${config.family} in ${dynamipsImageDir}`);
  return path.join(dynamipsImageDir, fileName);
}

async function createDynamipsNode(project, config, name) {
  const body = {
    node_type: "dynamips",
    name,
    x: 0,
    y: 0,
    image: imageForFamily(config),
    platform: config.platform,
    chassis: config.chassis,
    ram: config.ram,
    nvram: config.nvram,
    idlepc: config.idlepc,
    slot0: config.slot0,
    slot1: config.slot1,
    slot2: config.slot2,
    slot3: config.slot3,
    slot4: config.slot4,
    wic0: config.wic0,
    wic1: config.wic1,
    wic2: config.wic2,
    npe: config.npe,
    midplane: config.midplane,
    iomem: config.iomem,
    mmap: true,
    sparsemem: config.sparsemem,
    clock_divisor: Number(process.env.NETWORK_SIMULATOR_TEST_CLOCK_DIVISOR ?? 8),
    idlemax: Number(process.env.NETWORK_SIMULATOR_TEST_IDLEMAX ?? 500),
    idlesleep: Number(process.env.NETWORK_SIMULATOR_TEST_IDLESLEEP ?? 30),
    exec_area: 64,
    disk0: 0,
    disk1: 0,
    ghost_ios: false,
    console_type: "telnet",
    aux_type: "none",
    startup_config: "",
    adapters: 1,
  };
  for (const key of Object.keys(body)) if (body[key] === undefined) delete body[key];
  const node = await api(`/v3/projects/${project}/nodes`, { method: "POST", body });
  if (node.runtime_working_dir) runtimeDirectories.add(node.runtime_working_dir);
  return node;
}

async function createLink(project, left, right) {
  return api(`/v3/projects/${project}/links`, {
    method: "POST",
    body: { nodes: [left, right], link_type: "ethernet" },
  });
}

function endpoint(node, port = 0) {
  return { node_id: node.node_id, adapter_number: 0, port_number: port };
}

function matchingDynamips(port) {
  const output = execFileSync("ps", ["-eo", "pid=,pcpu=,args="], { encoding: "utf8" });
  return output.split("\n").map((line) => line.trim()).filter((line) => line.includes(`127.0.0.1:${port}`));
}

function processCpuTicks(pid) {
  const taskDirectory = `/proc/${pid}/task`;
  return fs.readdirSync(taskDirectory).reduce((total, threadId) => {
    try {
      const stat = fs.readFileSync(`${taskDirectory}/${threadId}/stat`, "utf8");
      const fields = stat.slice(stat.lastIndexOf(")") + 1).trim().split(/\s+/);
      return total + Number(fields[11]) + Number(fields[12]);
    } catch {
      return total;
    }
  }, 0);
}

async function sampleDynamipsCpu(port, durationMs = 2500) {
  const row = matchingDynamips(port)[0];
  assert.ok(row, `no Dynamips process found on hypervisor port ${port}`);
  const pid = row.split(/\s+/)[0];
  const ticksPerSecond = Number(execFileSync("getconf", ["CLK_TCK"], { encoding: "utf8" }).trim());
  const startTicks = processCpuTicks(pid);
  const start = Date.now();
  await sleep(durationMs);
  const elapsedSeconds = (Date.now() - start) / 1000;
  const consumedTicks = processCpuTicks(pid) - startTicks;
  return Number((consumedTicks / ticksPerSecond / elapsedSeconds * 100).toFixed(1));
}

function listFilesRecursively(directory) {
  if (!fs.existsSync(directory)) return [];
  return fs.readdirSync(directory, { withFileTypes: true }).flatMap((entry) => {
    const path = `${directory}/${entry.name}`;
    return entry.isDirectory() ? listFilesRecursively(path) : [path];
  });
}

async function main() {
  const health = await api("/healthz");
  assert.equal(health.status, "ok");
  const templates = await api("/v3/templates");
  const project = await api("/v3/projects", { method: "POST", body: { name: `Live IOS lab ${new Date().toISOString()}` } });
  projectId = project.project_id;

  const routers = new Map();
  for (const config of selectedFamilyCases) {
    routers.set(config.family, await createDynamipsNode(projectId, config, config.family.toUpperCase()));
  }
  const hasFullTopology = ["c3725", "c7200", "c3620"].every((family) => routers.has(family));
  const r1 = routers.get("c3725");
  const r2 = routers.get("c7200");
  const r3 = routers.get("c3620");
  let v1;
  let l1;
  if (hasFullTopology) {
    v1 = await api(`/v3/projects/${projectId}/nodes`, { method: "POST", body: { node_type: "vpcs", name: "VPC1", adapters: 1 } });
    await setStartupAddress(projectId, r1, "10.20.0.1", "255.255.255.252");
    await setStartupAddress(projectId, r2, "10.20.0.2", "255.255.255.252");
    await setStartupAddress(projectId, r3, "10.30.0.1");
    l1 = await createLink(projectId, endpoint(r1), endpoint(r2));
    await createLink(projectId, endpoint(r3), endpoint(v1));
  }

  const routerConsoles = new Map();
  for (const config of selectedFamilyCases) {
    const router = routers.get(config.family);
    await api(`/v3/projects/${projectId}/nodes/${router.node_id}/start`, { method: "POST" });
    let console = await bootIos(projectId, router);
    await sleep(4000);
    const cpuBefore = await sampleDynamipsCpu(router.runtime_hypervisor_port, 5000);
    let proposals;
    let automatic;
    if (config.family === "c3725") {
      proposals = await api(`/v3/projects/${projectId}/nodes/${router.node_id}/dynamips/idlepc_proposals`);
      assert.ok(Array.isArray(proposals) && proposals.length > 0, "Dynamips returned no Idle-PC proposals");
    }
    if (config.family === "c3725" || cpuBefore >= 10) {
      automatic = await api(`/v3/projects/${projectId}/nodes/${router.node_id}/dynamips/auto_idlepc`);
      assert.match(automatic.idlepc, /^0x[0-9a-f]+$/i);
      await sleep(1000);
    }
    const cpuAfter = automatic
      ? await sampleDynamipsCpu(router.runtime_hypervisor_port, 5000)
      : cpuBefore;
    assert.ok(cpuAfter < 10, `${config.family} idle-PC CPU was ${cpuAfter}% of one core after apply`);
    record(
      `IOS boot ${config.family}`,
      `IOS prompt reached; image=${path.basename(router.image)}; idlepc=${automatic?.idlepc ?? router.idlepc ?? "unset"}; process CPU=${cpuAfter}% of one core${automatic ? ` (before auto-selection ${cpuBefore}%)` : ""}`,
    );

    if (automatic) {
      const persisted = await api(`/v3/projects/${projectId}/nodes/${router.node_id}`);
      assert.equal(persisted.idlepc, automatic.idlepc, `${config.family} auto-selected Idle-PC was not persisted`);
    }
    if (config.family === "c3725") {
      const selected = automatic?.idlepc ?? router.idlepc;
      assert.ok(selected, "c3725 must have an Idle-PC value");
      await api(`/v3/projects/${projectId}/nodes/${router.node_id}/stop`, { method: "POST" });
      console.close();
      await api(`/v3/projects/${projectId}/nodes/${router.node_id}/start`, { method: "POST" });
      console = await bootIos(projectId, router);
      await sleep(5000);
      const restartedCpu = await sampleDynamipsCpu(router.runtime_hypervisor_port, 8000);
      assert.ok(restartedCpu < 10, `c3725 persisted Idle-PC CPU was ${restartedCpu}% after restart`);
      routerConsoles.set(router.node_id, console);
      record("Idle-PC restart persistence", `selected=${selected}; persisted router CPU=${restartedCpu}% of one core after restart`);
    } else if (["c3725", "c7200", "c3620"].includes(config.family)) {
      routerConsoles.set(router.node_id, console);
    } else {
      console.close();
      await api(`/v3/projects/${projectId}/nodes/${router.node_id}/stop`, { method: "POST" });
    }
  }

  // Filtered runs measure the selected router families independently. The
  // traffic topology below requires all three of its fixed router families.
  if (!hasFullTopology) return;

  for (const router of [r1, r2]) await iosCommand(routerConsoles.get(router.node_id), "enable", /#\s*$/m);
  let output = await iosPing(routerConsoles.get(r1.node_id), "10.20.0.2");
  assert.match(output, /Success rate is\s+100 percent/i, "router-to-router ping failed");
  record("router-to-router ping", "c3725 ↔ c7200, 5/5 IOS probes");
  await iosCommand(routerConsoles.get(r1.node_id), "enable", /#\s*$/m);

  // Exercise deletion and recreation while both router hypervisors are live.
  await api(`/v3/projects/${projectId}/links/${l1.link_id}`, { method: "DELETE" });
  await createLink(projectId, endpoint(r1), endpoint(r2));
  await api(`/v3/projects/${projectId}/nodes/${r2.node_id}/reload`, { method: "POST" });
  routerConsoles.get(r2.node_id).close();
  routerConsoles.set(r2.node_id, await bootIos(projectId, r2));
  await iosCommand(routerConsoles.get(r2.node_id), "enable", /#\s*$/m);
  output = await iosPing(routerConsoles.get(r1.node_id), "10.20.0.2");
  assert.match(output, /Success rate is\s+100 percent/i, "router ping failed after reload and link recreation");
  await api(`/v3/projects/${projectId}/nodes/${r2.node_id}/suspend`, { method: "POST" });
  const suspended = await api(`/v3/projects/${projectId}/nodes/${r2.node_id}`);
  assert.equal(suspended.status, "suspended");
  await api(`/v3/projects/${projectId}/nodes/${r2.node_id}/start`, { method: "POST" });
  record("router lifecycle and running link mutation", "router reload, suspend/resume, delete/create link, and post-reload ping passed");

  // R3 and VPC1 use a dedicated point-to-point subnet.
  await api(`/v3/projects/${projectId}/nodes/${r3.node_id}/start`, { method: "POST" });
  const r3Console = await bootIos(projectId, r3);
  await iosCommand(r3Console, "enable", /#\s*$/m);
  await api(`/v3/projects/${projectId}/nodes/${v1.node_id}/start`, { method: "POST" });
  const v1Console = await vpcsConsole(projectId, v1);
  await vpcsCommand(v1Console, "ip 10.30.0.2/24 10.30.0.1");
  const vpcsPing = await pingVpcs(v1Console, "10.30.0.1");
  assert.match(vpcsPing, /(?:bytes from|icmp_seq=|reply from)/i, "VPCS did not report router ping response");
  output = await iosPing(r3Console, "10.30.0.2");
  assert.match(output, /Success rate is\s+100 percent/i, "router-to-VPCS ping failed");
  record("router-to-VPCS ping", "c3600/3620 ↔ VPCS, IOS and VPCS console probes succeeded");

  // Replace the point-to-point VPCS link with a 3-host Ethernet-switch segment.
  await api(`/v3/projects/${projectId}/links/${await findLinkId(projectId, v1.node_id)}`, { method: "DELETE" });
  const switchTemplate = templates.find((entry) => entry.template_type === "ethernet_switch");
  const sw = await api(`/v3/projects/${projectId}/templates/${switchTemplate.template_id}`, { method: "POST", body: { name: "LAN switch" } });
  const v2 = await api(`/v3/projects/${projectId}/nodes`, { method: "POST", body: { node_type: "vpcs", name: "VPC2", adapters: 1 } });
  const docker = await api(`/v3/projects/${projectId}/nodes`, { method: "POST", body: { node_type: "docker", name: "Alpine host", image: "alpine:latest", adapters: 1, memory: 64, start_command: "/bin/sh" } });
  await api(`/v3/projects/${projectId}/links`, { method: "POST", body: { nodes: [endpoint(r3), endpoint(sw, 0)], link_type: "ethernet" } });
  await api(`/v3/projects/${projectId}/links`, { method: "POST", body: { nodes: [endpoint(v2), endpoint(sw, 1)], link_type: "ethernet" } });
  await api(`/v3/projects/${projectId}/links`, { method: "POST", body: { nodes: [endpoint(docker), endpoint(sw, 2)], link_type: "ethernet" } });
  await api(`/v3/projects/${projectId}/nodes/${v1.node_id}/stop`, { method: "POST" });
  await api(`/v3/projects/${projectId}/nodes/${v2.node_id}/start`, { method: "POST" });
  await api(`/v3/projects/${projectId}/nodes/${docker.node_id}/start`, { method: "POST" });
  const v2Console = await vpcsConsole(projectId, v2);
  await vpcsCommand(v2Console, "ip 10.30.0.2/24 10.30.0.1");
  const dockerConsole = openConsole(`/v3/compute/projects/${projectId}/docker/nodes/${docker.node_id}/console/ws`);
  await dockerConsole.ready();
  dockerConsole.send("ip addr add 10.30.0.3/24 dev eth0; ip link set eth0 up\r");
  await sleep(1000);
  dockerConsole.send("ping -c 3 -W 2 10.30.0.1\r");
  const dockerPing = await dockerConsole.wait(/3 packets transmitted|bytes from|Network is unreachable|100% packet loss/i, 15_000, "Docker ping output");
  assert.match(dockerPing.content, /3 packets transmitted, 3 packets received|bytes from/i, "Docker host did not reach router through switch");
  output = await iosPing(r3Console, "10.30.0.2");
  assert.match(output, /Success rate is\s+100 percent/i, "router could not reach VPCS through switch");
  output = await iosPing(r3Console, "10.30.0.3");
  assert.match(output, /Success rate is\s+100 percent/i, "router could not reach Docker through switch");
  record("router + VPCS + Docker switch segment", "c3600/3620, VPCS, and Alpine host exchanged IPv4 traffic through Ethernet switch");

  const nodeList = await api(`/v3/projects/${projectId}/nodes`);
  const r1After = nodeList.find((node) => node.node_id === r1.node_id);
  const processRows = matchingDynamips(r1.runtime_hypervisor_port);
  record("Dynamips CPU/process check", `runtime hypervisor ${r1.runtime_hypervisor_port}: ${JSON.stringify(processRows)}; IOS idle-PC ${r1After?.idlepc}`);

  console.log(JSON.stringify({ projectId, passed: results.length, results, runtimeDirectories: [...runtimeDirectories] }, null, 2));
}

async function findLinkId(project, nodeId) {
  const links = await api(`/v3/projects/${project}/links`);
  const link = links.find((item) => item.nodes?.some((entry) => entry.node_id === nodeId));
  assert.ok(link, `no link attached to ${nodeId}`);
  return link.link_id;
}

try {
  await main();
} finally {
  for (const socket of consoles) try { socket.close(); } catch {}
  if (projectId) {
    try {
      const nodeRows = await api(`/v3/projects/${projectId}/nodes`);
      for (const node of nodeRows) {
        if (!node.runtime_working_dir) continue;
        runtimeDirectories.add(node.runtime_working_dir);
        for (const file of ["dynamips_stdout.log", "dynamips_engine.log"]) {
          const path = `${node.runtime_working_dir}/${file}`;
          if (fs.existsSync(path)) {
            console.error(`RUNTIME LOG ${path}:\n${fs.readFileSync(path, "utf8").slice(-12000)}`);
          }
        }
      }
      await api(`/v3/projects/${projectId}`, { method: "DELETE" });
      record("live project cleanup", "project deleted through v3 API; runtime lifecycle cleanup ran");
      const orphaned = nodeRows.flatMap((node) => node.runtime_hypervisor_port
        ? matchingDynamips(node.runtime_hypervisor_port)
        : []);
      assert.deepEqual(orphaned, [], "project deletion must leave no tracked Dynamips hypervisor process");
      record("Dynamips process cleanup", "no test hypervisor process remained after project deletion");
      const residualFiles = [...runtimeDirectories].flatMap(listFilesRecursively);
      const transientFiles = residualFiles.filter((path) => /(?:\.log|\.pid|\.sock|\.tmp|\/\.dynamips-runtime|\/ilt_[^/]+)$/.test(path));
      assert.deepEqual(transientFiles, [], "project cleanup must remove Dynamips logs and transient runtime files");
      console.log(JSON.stringify({ residualRuntimeFiles: residualFiles }));
    } catch (error) {
      console.error(`CLEANUP ERROR: ${error.message}`);
      process.exitCode = 1;
    }
  }
  const residualDirectories = [...runtimeDirectories].filter((path) => {
    return fs.existsSync(path);
  });
  assert.deepEqual(residualDirectories, [], "project cleanup must remove Dynamips runtime directories when no virtual disks remain");
  console.log(JSON.stringify({ residualRuntimeDirectories: residualDirectories }));
}
