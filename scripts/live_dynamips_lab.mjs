#!/usr/bin/env node

// Opt-in live IOS integration scenario. This intentionally uses the controller's
// v3 routes and globally installed image library. It never writes image data.

import assert from "node:assert/strict";
import fs from "node:fs";
import { setTimeout as sleep } from "node:timers/promises";
import { execFileSync } from "node:child_process";

const base = (process.env.NETWORK_SIMULATOR_BASE_URL ?? "http://127.0.0.1:3082").replace(/\/$/, "");
const results = [];
let projectId;
const consoles = new Set();
const runtimeDirectories = new Set();

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
    if (/^[A-Za-z0-9_.-]+[>#]\s*$/m.test(output)) {
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

async function createNodeFromTemplate(project, templates, platform, name) {
  const template = templates.find((candidate) => candidate.template_type === "dynamips" && candidate.platform === platform);
  assert.ok(template, `no imported Dynamips appliance template for ${platform}`);
  const node = await api(`/v3/projects/${project}/templates/${template.template_id}`, {
    method: "POST", body: { name, x: 0, y: 0 },
  });
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
  for (const platform of ["c3600", "c3725", "c7200"]) {
    assert.ok(templates.some((entry) => entry.template_type === "dynamips" && entry.platform === platform), `missing ${platform} appliance template`);
  }

  const project = await api("/v3/projects", { method: "POST", body: { name: `Live IOS lab ${new Date().toISOString()}` } });
  projectId = project.project_id;
  const r1 = await createNodeFromTemplate(projectId, templates, "c3725", "R1");
  const r2 = await createNodeFromTemplate(projectId, templates, "c7200", "R2");
  const r3 = await createNodeFromTemplate(projectId, templates, "c3600", "R3");
  const v1 = await api(`/v3/projects/${projectId}/nodes`, { method: "POST", body: { node_type: "vpcs", name: "VPC1", adapters: 1 } });
  await setStartupAddress(projectId, r1, "10.20.0.1", "255.255.255.252");
  await setStartupAddress(projectId, r2, "10.20.0.2", "255.255.255.252");
  await setStartupAddress(projectId, r3, "10.30.0.1");
  const l1 = await createLink(projectId, endpoint(r1), endpoint(r2));
  await createLink(projectId, endpoint(r3), endpoint(v1));

  const routerConsoles = new Map();
  const bootOnly = process.env.NETWORK_SIMULATOR_LIVE_BOOT_ONLY;
  const bootTargets = bootOnly
    ? [new Map([["c3725", r1], ["c7200", r2], ["c3600", r3]]).get(bootOnly)].filter(Boolean)
    : [r1, r2];
  assert.ok(bootTargets.length > 0, `unknown NETWORK_SIMULATOR_LIVE_BOOT_ONLY platform: ${bootOnly}`);
  for (const router of bootTargets) {
    await api(`/v3/projects/${projectId}/nodes/${router.node_id}/start`, { method: "POST" });
    routerConsoles.set(router.node_id, await bootIos(projectId, router));
  }
  if (bootOnly) {
    record("focused IOS boot", `${bootOnly} template reached the IOS CLI`);
    return;
  }
  for (const router of [r1, r2]) await iosCommand(routerConsoles.get(router.node_id), "enable", /#\s*$/m);
  let output = await iosPing(routerConsoles.get(r1.node_id), "10.20.0.2");
  assert.match(output, /Success rate is\s+100 percent/i, "router-to-router ping failed");
  record("router-to-router ping", "c3725 ↔ c7200, 5/5 IOS probes");

  const proposals = await api(`/v3/projects/${projectId}/nodes/${r1.node_id}/dynamips/idlepc_proposals`);
  assert.ok(Array.isArray(proposals) && proposals.length > 0, "Dynamips returned no Idle-PC proposals");
  const automatic = await api(`/v3/projects/${projectId}/nodes/${r1.node_id}/dynamips/auto_idlepc`);
  assert.match(automatic.idlepc, /^0x[0-9a-f]+$/i);
  const persistedIdlePc = await api(`/v3/projects/${projectId}/nodes/${r1.node_id}`);
  assert.equal(persistedIdlePc.idlepc, automatic.idlepc, "auto-selection route must persist the chosen Idle-PC");
  const cpuBefore = await sampleDynamipsCpu(r1.runtime_hypervisor_port);
  assert.ok(cpuBefore < 50, `selected Idle-PC CPU remained high before restart (${cpuBefore}%)`);
  await api(`/v3/projects/${projectId}/nodes/${r1.node_id}/stop`, { method: "POST" });
  await api(`/v3/projects/${projectId}/nodes/${r1.node_id}/start`, { method: "POST" });
  routerConsoles.get(r1.node_id).close();
  routerConsoles.set(r1.node_id, await bootIos(projectId, r1));
  await iosCommand(routerConsoles.get(r1.node_id), "enable", /#\s*$/m);
  // Let IOS finish its console/login work before measuring steady-state
  // process CPU; the persisted value is applied during router startup.
  await sleep(5000);
  const cpuAfter = await sampleDynamipsCpu(r1.runtime_hypervisor_port, 8000);
  assert.ok(cpuAfter < 50, `persisted Idle-PC CPU remained high after restart (${cpuAfter}%)`);
  record("Idle-PC", `proposals=${proposals.length}, selected=${automatic.idlepc}, persisted and restarted; Dynamips process CPU before=${cpuBefore}% after=${cpuAfter}%`);

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
  record("router-to-VPCS ping", "c3600 ↔ VPCS, IOS and VPCS console probes succeeded");

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
  record("router + VPCS + Docker switch segment", "c3600, VPCS, and Alpine host exchanged IPv4 traffic through Ethernet switch");

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
