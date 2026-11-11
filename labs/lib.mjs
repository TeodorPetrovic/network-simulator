// Declarative lab builder shared by every lab definition and by scripts/labs.mjs.
// A lab module exports { id, title, summary, needs, build(lab), verify(lab, t) }.
import { execFileSync } from "node:child_process";

const sleep = (ms) => new Promise((resolve) => setTimeout(resolve, ms));
export { sleep };

export function client(base, token) {
  const api = async (method, path, body) => {
    const res = await fetch(`${base}/v3${path}`, {
      method,
      headers: { "content-type": "application/json", ...(token ? { authorization: `Bearer ${token}` } : {}) },
      body: body === undefined ? undefined : JSON.stringify(body),
    });
    const text = await res.text();
    let json; try { json = JSON.parse(text); } catch { json = text; }
    return { status: res.status, json };
  };
  return { base, token, api };
}

export async function login(base, user, password) {
  const res = await fetch(`${base}/v3/access/users/authenticate`, {
    method: "POST", headers: { "content-type": "application/json" }, body: JSON.stringify({ username: user, password }),
  });
  if (!res.ok) throw new Error(`login failed: ${res.status} ${await res.text()}`);
  return (await res.json()).access_token;
}

/** Shell snippet that waits for ethN to exist before the rest of a start command runs. */
export const waitIf = (count) => Array.from({ length: count }, (_, i) => `while ! ip link show eth${i} >/dev/null 2>&1; do sleep 1; done`).join("; ");

/** Wrap a script into a docker start command that survives the gap before NICs are attached. */
export function startScript(adapters, script) {
  const body = `${waitIf(adapters)}; ${script}`.replace(/'/g, `'\\''`);
  return `/bin/sh -c '${body}'`;
}

/** Alpine/Linux host start command: addresses, routes, then idle. */
export function hostStart({ ip, gw, adapters = 1, extra = "", vlan }) {
  const parts = [];
  parts.push("ip link set lo up");
  parts.push("ip link set eth0 up");
  if (vlan) { parts.push(`ip link add link eth0 name eth0.${vlan} type vlan id ${vlan}`, `ip link set eth0.${vlan} up`, `ip addr add ${ip} dev eth0.${vlan}`); }
  else if (ip) parts.push(`ip addr add ${ip} dev eth0`);
  if (gw) parts.push(`ip route add default via ${gw}`);
  if (extra) parts.push(extra);
  parts.push("exec sleep infinity");
  return startScript(adapters, parts.join("; ").replace(/&; /g, "& "));
}

export class Lab {
  constructor(cli, name, { description = "" } = {}) {
    this.cli = cli; this.name = name; this.description = description;
    this.nodes = new Map(); this.projectId = null; this.consoles = [];
  }
  api(method, path, body) { return this.cli.api(method, path, body); }
  async create() {
    const existing = (await this.api("GET", "/projects")).json;
    for (const project of Array.isArray(existing) ? existing : []) {
      if (project.name === this.name) await this.destroyProject(project.project_id);
    }
    const res = await this.api("POST", "/projects", { name: this.name });
    if (res.status !== 201) throw new Error(`create project: ${JSON.stringify(res)}`);
    this.projectId = res.json.project_id;
    return this;
  }
  async destroyProject(id) {
    await this.api("POST", `/projects/${id}/close`, {});
    await this.api("DELETE", `/projects/${id}`);
  }
  async addNode(key, body) {
    const res = await this.api("POST", `/projects/${this.projectId}/nodes`, { compute_id: "local", ...body, name: body.name ?? key });
    if (res.status !== 201) throw new Error(`add node ${key}: ${JSON.stringify(res)}`);
    res.json.api_base = this.cli.base;
    this.nodes.set(key, res.json);
    await this.tag(key);
    return res.json;
  }
  /** Remember the lab key in the node's usage text so an imported project can be re-attached. */
  async tag(key) {
    const node = this.node(key);
    await this.api("PUT", `/projects/${this.projectId}/nodes/${node.node_id}`, { usage: `lab-key:${key}` });
  }
  /** Rebuild the key -> node map of an already imported project. */
  async attach() {
    const res = await this.api("GET", `/projects/${this.projectId}/nodes`);
    // GNS3's file format has no free-form node field, so archives carry a name -> key map as a
    // project file (ignored by real GNS3); fall back to the build-time `usage` tag.
    const file = await fetch(`${this.cli.base}/v3/projects/${this.projectId}/files/lab-keys.json`, {
      headers: this.cli.token ? { authorization: `Bearer ${this.cli.token}` } : {},
    });
    const byName = file.ok ? await file.json().catch(() => ({})) : {};
    this.nodes.clear();
    for (const node of res.json) {
      const key = byName[node.name] ?? /^lab-key:(.+)$/.exec(node.usage ?? "")?.[1];
      if (key) this.nodes.set(key, { ...node, api_base: this.cli.base });
    }
  }
  /** Record which node plays which lab role, for archives (see attach()). */
  async saveKeys() {
    const map = Object.fromEntries([...this.nodes].map(([key, node]) => [node.name, key]));
    const res = await fetch(`${this.cli.base}/v3/projects/${this.projectId}/files/lab-keys.json`, {
      method: "POST",
      headers: { "content-type": "application/octet-stream", ...(this.cli.token ? { authorization: `Bearer ${this.cli.token}` } : {}) },
      body: JSON.stringify(map),
    });
    if (!res.ok) throw new Error(`write lab-keys.json: ${res.status}`);
  }
  async findLink(a, b) {
    const ids = [this.node(a).node_id, this.node(b).node_id];
    const links = (await this.api("GET", `/projects/${this.projectId}/links`)).json;
    return links.find((l) => ids.every((id) => l.nodes.some((n) => n.node_id === id)));
  }
  docker(key, { image = "alpine:latest", adapters = 1, start, x = 0, y = 0, env, console: consoleType, vncPort, httpPort, name } = {}) {
    const body = { name, node_type: "docker", image, adapters, x, y };
    if (start) body.start_command = start;
    if (env) body.environment = Object.entries(env).map(([k, v]) => `${k}=${v}`).join("\n");
    if (consoleType) body.console_type = consoleType;
    if (vncPort) body.console_vnc_port = vncPort;
    if (httpPort) body.console_http_port = httpPort;
    return this.addNode(key, body);
  }
  switch(key, { ports = 8, x = 0, y = 0, name } = {}) {
    return this.addNode(key, { name, node_type: "ethernet_switch", x, y });
  }
  hub(key, { x = 0, y = 0, name } = {}) { return this.addNode(key, { name, node_type: "ethernet_hub", x, y }); }
  nat(key, { x = 0, y = 0, name = "NAT" } = {}) { return this.addNode(key, { name, node_type: "nat", x, y }); }
  vpcs(key, { x = 0, y = 0, name } = {}) { return this.addNode(key, { name, node_type: "vpcs", x, y }); }
  /** A VPCS end host with its address baked into the startup script (`ip <addr>/<len> [gateway]`). */
  async pc(key, { name, x = 0, y = 0, ip, gw } = {}) {
    await this.vpcs(key, { name, x, y });
    await this.configure(key, { startup_script: `ip ${ip}${gw ? ` ${gw}` : ""}` });
  }
  /** Ping from a VPCS node's console until replies arrive; returns the console output. */
  async pcPing(key, target, { tries = 8, count = 3 } = {}) {
    const con = await telnet(this.node(key));
    let out = "";
    try {
      for (let i = 0; i < tries && !/bytes from .*ttl=\d+/.test(out); i += 1) {
        con.send("\r"); con.clear(); con.send(`ping ${target} -c ${count}\r`);
        await sleep(count * 1500 + 3000);
        out = con.output;
      }
    } finally { con.close(); }
    return out;
  }
  /** Ensure a (global) template exists and place a node from it. */
  async template(spec) {
    const list = (await this.api("GET", "/templates")).json;
    const found = list.find((tpl) => tpl.name === spec.name);
    if (found) return found;
    const res = await this.api("POST", "/templates", spec);
    if (res.status !== 201) throw new Error(`template ${spec.name}: ${JSON.stringify(res)}`);
    return res.json;
  }
  async fromTemplate(key, tpl, { x = 0, y = 0, name } = {}) {
    const res = await this.api("POST", `/projects/${this.projectId}/templates/${tpl.template_id}`, { x, y, compute_id: "local", ...(name ? { name } : {}) });
    if (res.status !== 201) throw new Error(`node from ${tpl.name}: ${JSON.stringify(res)}`);
    res.json.api_base = this.cli.base;
    this.nodes.set(key, res.json);
    await this.tag(key);
    return res.json;
  }
  /** Change runtime/port settings later (switch ports_mapping, adapters, ...) via PUT /nodes/{id}, like GNS3. */
  async configure(key, patch) {
    const res = await this.api("PUT", `/projects/${this.projectId}/nodes/${this.node(key).node_id}`, patch);
    if (res.status !== 200) throw new Error(`configure ${key}: ${JSON.stringify(res)}`);
    this.nodes.set(key, { ...this.node(key), ...res.json, api_base: this.cli.base });
    return res.json;
  }
  node(key) { const node = this.nodes.get(key); if (!node) throw new Error(`unknown node ${key}`); return node; }
  endpoint(spec) {
    const [key, idx = 0] = Array.isArray(spec) ? spec : [spec, 0];
    const node = this.node(key);
    if (node.node_type === "ethernet_switch" || node.node_type === "ethernet_hub") return { node_id: node.node_id, adapter_number: 0, port_number: Number(idx) };
    if (typeof idx === "string" && idx.includes("/")) { const [a, p] = idx.split("/").map(Number); return { node_id: node.node_id, adapter_number: a, port_number: p }; }
    if (node.node_type === "docker") return { node_id: node.node_id, adapter_number: 0, port_number: Number(idx) };
    return { node_id: node.node_id, adapter_number: Number(idx), port_number: 0 };
  }
  async link(a, b) {
    const res = await this.api("POST", `/projects/${this.projectId}/links`, { nodes: [this.endpoint(a), this.endpoint(b)] });
    if (res.status !== 201) throw new Error(`link ${JSON.stringify(a)}-${JSON.stringify(b)}: ${JSON.stringify(res)}`);
    return res.json;
  }
  async note(text, x, y) {
    const svg = `<svg height="60" width="420"><text fill="#888" font-family="monospace" font-size="12" x="4" y="14">${text.split("\n").map((l, i) => `<tspan x="4" dy="${i ? 16 : 0}">${l.replace(/&/g, "&amp;").replace(/</g, "&lt;")}</tspan>`).join("")}</text></svg>`;
    await this.api("POST", `/projects/${this.projectId}/drawings`, { x, y, z: 1, svg });
  }
  async refresh() {
    const res = await this.api("GET", `/projects/${this.projectId}/nodes`);
    for (const fresh of res.json) for (const [key, node] of this.nodes) if (node.node_id === fresh.node_id) this.nodes.set(key, { ...node, ...fresh, api_base: this.cli.base });
  }
  async open() { await this.api("POST", `/projects/${this.projectId}/open`, {}); }
  async startAll(all = [...this.nodes.keys()], { concurrency = 4, timeout = 120000 } = {}) {
    const keys = all.filter((key) => !["ethernet_switch", "ethernet_hub"].includes(this.node(key).node_type));
    const queue = [...keys];
    const worker = async () => {
      while (queue.length) {
        const key = queue.shift(); const node = this.node(key);
        const res = await this.api("POST", `/projects/${this.projectId}/nodes/${node.node_id}/start`, {});
        if (res.status !== 200) throw new Error(`start ${key}: ${JSON.stringify(res)}`);
      }
    };
    await Promise.all(Array.from({ length: concurrency }, worker));
    const end = Date.now() + timeout;
    for (;;) {
      await this.refresh();
      if (keys.every((key) => this.node(key).status === "started")) return;
      if (Date.now() > end) throw new Error(`nodes not started: ${keys.filter((k) => this.node(k).status !== "started")}`);
      await sleep(500);
    }
  }
  async stopAll() {
    for (const [, node] of [...this.nodes].reverse()) await this.api("POST", `/projects/${this.projectId}/nodes/${node.node_id}/stop`, {}).catch(() => {});
  }
  /** docker exec inside a docker node; returns stdout (throws on non-zero exit). */
  exec(key, command, { timeout = 60000 } = {}) {
    const node = this.node(key);
    return execFileSync("docker", ["exec", node.container_id, "sh", "-c", command], { encoding: "utf8", timeout });
  }
  /** Retry an exec until its output matches (or throws at the deadline). */
  async expectExec(key, command, regex, { timeout = 60000, interval = 2000 } = {}) {
    const end = Date.now() + timeout; let last = "";
    for (;;) {
      try { last = this.exec(key, command); if (regex.test(last)) return last; } catch (error) { last = String(error.stdout ?? error.message); }
      if (Date.now() > end) throw new Error(`${key}: \`${command}\` never matched ${regex}; last output:\n${last}`);
      await sleep(interval);
    }
  }
}

/** Start command for the GNS3 Open vSwitch image: brings NICs up, runs its init (br0 gets every ethN
 *  except `mgmt`), applies extra shell lines, then idles. */
export function ovsStart(adapters, lines = [], { mgmt = "none" } = {}) {
  return startScript(adapters, [
    ...Array.from({ length: adapters }, (_, i) => `ip link set eth${i} up`),
    `MANAGEMENT_INTERFACE=${mgmt} /etc/openvswitch/init.sh >/tmp/ovs-init.log 2>&1 || true`,
    ...lines,
    "exec sleep infinity",
  ].join("; "));
}

// ---------------------------------------------------------------- consoles

import net from "node:net";

function stripTelnet(buffer) {
  const out = [];
  for (let i = 0; i < buffer.length; i++) {
    if (buffer[i] !== 255) { out.push(buffer[i]); continue; }
    const cmd = buffer[i + 1];
    if (cmd === 255) { out.push(255); i += 1; } else if (cmd >= 251 && cmd <= 254) i += 2; else i += 1;
  }
  return Buffer.from(out).toString("latin1");
}

/** Raw telnet console used for Dynamips routers (node.console is a TCP port on the controller host). */
export async function telnet(node, timeout = 30000) {
  if (!node.console) return wsConsole(node, timeout);
  const socket = net.createConnection({ host: node.console_host && node.console_host !== "0.0.0.0" ? node.console_host : "127.0.0.1", port: Number(node.console) });
  let output = "";
  socket.on("data", (chunk) => { output += stripTelnet(chunk); });
  socket.on("error", () => {});
  await new Promise((resolve, reject) => { const timer = setTimeout(() => reject(new Error("console connect timeout")), timeout); socket.once("connect", () => { clearTimeout(timer); resolve(); }); socket.once("error", reject); });
  const api = {
    get output() { return output; },
    clear() { output = ""; },
    send(text) { socket.write(text); },
    async expect(regex, { timeout: ms = 20000, label = String(regex) } = {}) {
      const end = Date.now() + ms;
      while (!regex.test(output)) {
        if (Date.now() > end) throw new Error(`timeout waiting for ${label}; tail:\n${output.slice(-600)}`);
        await sleep(100);
      }
      return output;
    },
    close() { socket.destroy(); },
  };
  return api;
}

/** Wait for the IOS exec prompt, declining the initial configuration dialog. */
export async function bootIos(node, { timeout = 240000 } = {}) {
  const con = await telnet(node);
  const end = Date.now() + timeout;
  let answered = false; let nudged = 0;
  while (Date.now() < end) {
    const out = con.output;
    if (!answered && /initial configuration dialog[\s\S]{0,200}\[yes\/no\]/i.test(out)) { con.send("no\r"); answered = true; }
    else if (/press\s+return\s+to\s+get\s+started/i.test(out) && nudged < 20) { con.send("\r"); nudged += 1; con.clear(); await sleep(300); continue; }
    else if (/[A-Za-z0-9_.-]+[>#]\s*$/.test(out) && !/--More--/.test(out)) break;
    else if (Date.now() % 7000 < 150 && nudged < 40) { con.send("\r"); nudged += 1; }
    await sleep(150);
  }
  if (!/[A-Za-z0-9_.-]+[>#]\s*$/.test(con.output)) throw new Error(`IOS prompt never appeared on ${node.name}; tail:\n${con.output.slice(-500)}`);
  await sleep(500);
  const ios = {
    con,
    /** Run one command and return its output once a prompt is back. */
    async run(command, { timeout: ms = 30000 } = {}) {
      con.clear(); con.send(`${command}\r`);
      const end2 = Date.now() + ms;
      for (;;) {
        await sleep(120);
        if (/--More--\s*$/.test(con.output)) { con.send(" "); continue; }
        const clean = con.output.split(/\r?\n/).filter((l) => !/^\s*[*%]/.test(l)).join("\n");
        if (/\n[A-Za-z0-9_.-]+(\([a-z0-9-]+\))?[>#]\s*$/.test(clean)) return con.output;
        if (Date.now() > end2) throw new Error(`${node.name}: \`${command}\` timed out; tail:\n${con.output.slice(-400)}`);
      }
    },
    async config(lines) {
      await ios.run("enable").catch(() => {});
      await ios.run("configure terminal");
      for (const line of lines) await ios.run(line);
      await ios.run("end");
    },
    close() { con.close(); },
  };
  await ios.run("terminal length 0").catch(() => {});
  return ios;
}

// ---------------------------------------------------------------- Dynamips nodes

export const IOS_PLATFORMS = {
  c3725: { platform: "c3725", prefixes: ["c3725-", "ishare-16-c3725-"], ram: 128, nvram: 256, slot0: "GT96100-FE", idlepc: "0x60c09aa0", iomem: 5, sparsemem: true },
  c3745: { platform: "c3745", prefixes: ["c3745-", "ishare-17-c3745-"], ram: 256, nvram: 256, slot0: "GT96100-FE", idlepc: "0x60aa1da0", iomem: 5, sparsemem: true },
  c7200: { platform: "c7200", prefixes: ["c7200-", "ishare-22-c7200-"], ram: 512, nvram: 512, slot0: "C7200-IO-FE", npe: "npe-400", midplane: "vxr", idlepc: "0x6066a998", iomem: undefined, sparsemem: true },
};

import { readdirSync } from "node:fs";
export function iosImage(family) {
  const dir = `${process.env.NETWORK_SIMULATOR_IMAGE_HOME ?? `${process.env.HOME}/.local/share/network-simulator/images`}/dynamips`;
  const config = IOS_PLATFORMS[family];
  const file = readdirSync(dir).find((f) => f.endsWith(".image") && config.prefixes.some((p) => f.startsWith(p)));
  if (!file) throw new Error(`no ${family} IOS image in ${dir}`);
  return `${dir}/${file}`;
}

Lab.prototype.ios = function ios(key, { family = "c3725", name, x = 0, y = 0, slot1, slot2, disk0 = 0 } = {}) {
  const c = IOS_PLATFORMS[family];
  const body = {
    node_type: "dynamips", name: name ?? key, x, y, image: iosImage(family), platform: c.platform, chassis: c.chassis,
    ram: c.ram, nvram: c.nvram, idlepc: c.idlepc, slot0: c.slot0, slot1, slot2, npe: c.npe, midplane: c.midplane,
    iomem: c.iomem, mmap: true, sparsemem: c.sparsemem, clock_divisor: 8, idlemax: 500, idlesleep: 30, exec_area: 64,
    disk0, disk1: 0, ghost_ios: false, console_type: "telnet", aux_type: "none", startup_config: "", adapters: 1,
  };
  for (const k of Object.keys(body)) if (body[k] === undefined) delete body[k];
  return this.addNode(key, body);
};

// ---------------------------------------------------------------- MikroTik CHR

export const chrImage = `${process.env.NETWORK_SIMULATOR_IMAGE_HOME ?? `${process.env.HOME}/.local/share/network-simulator/images`}/qemu/mikrotik-chr-7.23.8.qcow2`;

Lab.prototype.chr = async function chr(key, { adapters = 2, name, x = 0, y = 0, memory = 256 } = {}) {
  const tpl = await this.template({
    name: `MikroTik CHR 7.23.8 ${adapters}-port (labs)`, template_type: "qemu", hda_disk_image: chrImage, adapters, memory, cpus: 1,
    platform: "x86_64", adapter_type: "e1000", hda_disk_interface: "ide", boot_priority: "c", linked_clone: true,
  });
  return this.fromTemplate(key, tpl, { x, y, name: name ?? key });
};

/** Log into a CHR serial console (admin / empty password) and return { run(line), close() }. */
export async function routerOs(lab, node, { timeout = 240000 } = {}) {
  const url = `${lab.cli.base.replace("http", "ws")}/v3/compute/projects/${lab.projectId}/qemu/nodes/${node.node_id}/console/ws`;
  const ws = new WebSocket(url);
  let output = "";
  ws.addEventListener("message", async (e) => {
    const text = typeof e.data === "string" ? e.data : e.data instanceof Blob ? await e.data.text() : new TextDecoder().decode(e.data);
    output += text;
    if (text.includes("\x1b[6n")) ws.send("\x1b[24;200R");
  });
  await new Promise((resolve, reject) => { ws.addEventListener("open", resolve, { once: true }); ws.addEventListener("error", reject, { once: true }); });
  const type = async (text) => { for (const c of text) { ws.send(c); await sleep(35); } };
  const expect = async (re, ms, label) => {
    const end = Date.now() + ms;
    while (!re.test(output)) { if (Date.now() > end) throw new Error(`${label}: timeout; tail ${JSON.stringify(output.slice(-300))}`); await sleep(200); }
  };
  const nudge = setInterval(() => { try { ws.send("\r"); } catch { /* closed */ } }, 4000);
  const prompt = /\[admin@[^\]]*\]\s*>\s*$/;
  try { await expect(/Login:|\[admin@[^\]]*\]\s*>/i, timeout, `${node.name} login prompt`); } finally { clearInterval(nudge); }
  if (!/Login:/i.test(output)) { await sleep(500); output = ""; ws.send("\r"); await sleep(500); return finish(); }
  await sleep(1500); output = "";
  await type("admin+ct200w\r");
  await expect(/assword:/i, 30000, `${node.name} password prompt`);
  output = ""; await type("\r");
  const end = Date.now() + 90000;
  while (!prompt.test(output)) {
    if (/software license\? \[Y\/n\]:\s*$/.test(output)) await type("n\r");
    if (/new password>\s*$/i.test(output)) { output = ""; ws.send("\x03"); await sleep(1500); ws.send("\r"); }
    if (Date.now() > end) throw new Error(`${node.name}: RouterOS prompt never appeared; tail ${JSON.stringify(output.slice(-300))}`);
    await sleep(300);
  }
  return finish();
  function finish() { return {
    async run(line, re = prompt, ms = 45000) { output = ""; await type(`${line}\r`); await expect(re, ms, `${node.name}: ${line}`); return output; },
    /** Gracefully power the router off so its configuration is flushed to disk. */
    async shutdown() { output = ""; await type("/system shutdown\r"); await sleep(1500); await type("y"); await sleep(4000); },
    close() { ws.close(); },
  }; }
}

/** Shell snippet that serves `body` over HTTP on :80 using busybox nc (Alpine has no httpd applet). */
export function httpServer(body) {
  const page = `<html><body>${body}</body></html>`;
  return `(while true; do printf 'HTTP/1.1 200 OK\\r\\nContent-Type: text/html\\r\\nContent-Length: ${page.length}\\r\\nConnection: close\\r\\n\\r\\n${page}' | nc -l -p 80 >/dev/null 2>&1; done) &`;
}

/** Console over the controller's WebSocket bridge (used by VPCS, which has no TCP console port). */
async function wsConsole(node, timeout) {
  const base = new URL(node.api_base);
  const kind = node.node_type === "qemu" ? "qemu" : node.node_type;
  const ws = new WebSocket(`${base.protocol === "https:" ? "wss:" : "ws:"}//${base.host}/v3/compute/projects/${node.project_id}/${kind}/nodes/${node.node_id}/console/ws`);
  let output = "";
  ws.addEventListener("message", async (e) => {
    output += typeof e.data === "string" ? e.data : e.data instanceof Blob ? await e.data.text() : new TextDecoder().decode(e.data);
  });
  await new Promise((resolve, reject) => {
    const timer = setTimeout(() => reject(new Error("console websocket timeout")), timeout);
    ws.addEventListener("open", () => { clearTimeout(timer); resolve(); }, { once: true });
    ws.addEventListener("error", () => { clearTimeout(timer); reject(new Error("console websocket failed")); }, { once: true });
  });
  return {
    get output() { return output; },
    clear() { output = ""; },
    send(text) { ws.send(text); },
    async expect(regex, { timeout: ms = 20000, label = String(regex) } = {}) {
      const end = Date.now() + ms;
      while (!regex.test(output)) { if (Date.now() > end) throw new Error(`timeout waiting for ${label}; tail:\n${output.slice(-600)}`); await sleep(100); }
      return output;
    },
    close() { ws.close(); },
  };
}

// ---------------------------------------------------------------- GNS3 appliance flow
// Labs are built the way a GNS3 user would: import the registry .gns3a through the app,
// install its image, create the template, then add devices from that template.

const maskOf = (prefix) => { const bits = (0xffffffff << (32 - prefix)) >>> 0; return [24, 16, 8, 0].map((s) => (bits >>> s) & 255).join("."); };

/** Render a busybox-ifupdown /etc/network/interfaces file. cfg: { eth0: { ip: "10.0.0.2/24", gw, dns, up: [] } | "dhcp" } */
export function interfaces(cfg) {
  const out = cfg.lo ? [] : ["auto lo", "iface lo inet loopback", ""];
  for (const [name, c] of Object.entries(cfg)) {
    out.push(`auto ${name}`);
    if (c === "dhcp") { out.push(`iface ${name} inet dhcp`, ""); continue; }
    if (!c.ip) { out.push(`iface ${name} inet manual`); for (const up of c.up ?? []) out.push(`\tup ${up}`); out.push(""); continue; }
    const [addr, prefix] = c.ip.split("/");
    out.push(`iface ${name} inet static`, `\taddress ${addr}`, `\tnetmask ${maskOf(Number(prefix))}`);
    if (c.gw) out.push(`\tgateway ${c.gw}`);
    for (const pre of c.pre ?? []) out.push(`\tpre-up ${pre}`);
    if (c.dns) out.push(`\tup echo nameserver ${c.dns} > /etc/resolv.conf`);
    for (const up of c.up ?? []) out.push(`\tup ${up}`);
    out.push("");
  }
  return `${out.join("\n")}\n`;
}

Lab.prototype.applianceTemplate = async function applianceTemplate(file, { version, image } = {}) {
  this.tplCache ??= new Map();
  const cacheKey = `${file}|${version ?? ""}`;
  if (this.tplCache.has(cacheKey)) return this.tplCache.get(cacheKey);
  const imp = await this.api("POST", `/marketplace/gns3/appliances/${file}/import`, {});
  if (imp.status >= 400 && imp.status !== 409) throw new Error(`import ${file}: ${JSON.stringify(imp)}`);
  const load = async () => (await this.api("GET", "/marketplace/gns3/manager")).json;
  let manager = await load();
  const app = manager.appliances.find((a) => a.file_name === file);
  if (!app) throw new Error(`appliance ${file} is not in the library after import`);
  const ver = (version && app.versions.find((v) => v.name === version)) || app.versions[0];
  if (app.template_type === "docker") {
    if (!ver.docker_image_available) {
      const pull = await this.api("POST", "/docker/images/pull", { image: ver.docker_image });
      if (pull.status !== 200) throw new Error(`pull ${ver.docker_image}: ${JSON.stringify(pull)}`);
    }
  } else if (app.template_type === "qemu") {
    for (const req of ver.images.filter((r) => r.status === "missing" || r.status === "mismatch")) {
      const local = manager.local_images.find((i) => i.template_type === "qemu" && (!image || i.name.includes(image)));
      if (!local) throw new Error(`no local QEMU image for ${file} ${req.key}`);
      const bind = await this.api("PUT", `/marketplace/gns3/appliances/${file}/bindings`, { version: ver.name, image_key: req.key, image_id: local.image_id, allow_checksum_mismatch: true });
      if (bind.status >= 300) throw new Error(`bind ${req.key}: ${JSON.stringify(bind)}`);
    }
  }
  const templates = (await this.api("GET", "/templates")).json;
  const tpl = templates.find((t) => t.appliance_id === app.appliance_id && (!t.appliance_version || t.appliance_version === ver.name || ver.name === "Default"))
    ?? (app.template_type === "dynamips" ? templates.find((t) => t.template_type === "dynamips" && String(t.image).includes(image ?? "c3725")) : undefined);
  if (!tpl) throw new Error(`appliance ${file} produced no template`);
  this.tplCache.set(cacheKey, tpl);
  return tpl;
};

Lab.prototype.putFile = async function putFile(key, path, text) {
  const res = await fetch(`${this.cli.base}/v3/projects/${this.projectId}/nodes/${this.node(key).node_id}/files/${path}`, {
    method: "POST", headers: { "content-type": "application/octet-stream", ...(this.cli.token ? { authorization: `Bearer ${this.cli.token}` } : {}) }, body: text,
  });
  if (!res.ok) throw new Error(`write ${path} on ${key}: ${res.status}`);
};

/** Add a device from an appliance: template -> node, then GNS3-style per-node edits. */
Lab.prototype.device = async function device(key, file, { version, image, x = 0, y = 0, name, set, net } = {}) {
  const tpl = await this.applianceTemplate(file, { version, image });
  const node = await this.fromTemplate(key, tpl, { x, y, name: name ?? key });
  if (set && Object.keys(set).length) await this.configure(key, set);
  if (net) await this.putFile(key, "etc/network/interfaces", interfaces(net));
  return node;
};

/** A local-image template (no appliance exists): pull when missing, then place from the image's template. */
Lab.prototype.imageDevice = async function imageDevice(key, imageRef, opts = {}) {
  // Pulling through the app also yields the template the app derives from the local image.
  const pull = await this.api("POST", "/docker/images/pull", { image: imageRef });
  if (pull.status !== 200) throw new Error(`pull ${imageRef}: ${JSON.stringify(pull)}`);
  // A pulled image is only a candidate; register it as a template like "New template from image".
  const { template_id: _id, builtin: _builtin, ...defaults } = pull.json.template;
  const tpl = await this.template({ ...defaults, name: `${imageRef} (labs)`, template_type: "docker", image: imageRef });
  const node = await this.fromTemplate(key, tpl, { x: opts.x ?? 0, y: opts.y ?? 0, name: opts.name ?? key });
  if (opts.set && Object.keys(opts.set).length) await this.configure(key, opts.set);
  if (opts.net) await this.putFile(key, "etc/network/interfaces", interfaces(opts.net));
  return node;
};

/** `/bin/sh -c '<script>'` start command for a node's start_command field. */
export function sh(script) { return `/bin/sh -c '${script.replace(/&;/g, "&").replace(/'/g, `'\\''`)}'`; }

/** Foreground busybox-nc HTTP responder (Alpine has no httpd applet). */
export function httpLoop(body) {
  const page = `<html><body>${body}</body></html>`;
  return sh(`while true; do printf 'HTTP/1.1 200 OK\\r\\nContent-Type: text/html\\r\\nContent-Length: ${page.length}\\r\\nConnection: close\\r\\n\\r\\n${page}' | nc -l -p 80 >/dev/null 2>&1; done`);
}

/** Start command for the registry Open vSwitch appliance: run its init, apply `lines`, then keep the shell. */
export function ovsCommand(lines) {
  const script = `/etc/openvswitch/init.sh; ${lines.join("; ")}; exec bash`;
  return `/bin/bash -c '${script.replace(/'/g, `'\\''`)}'`;
}
