// REST and raw-console helpers for live network scenarios.
import net from "node:net";
import { addNode, newProject, startServer, waitFor } from "../harness.mjs";

export { addNode, newProject, startServer, waitFor };

const sleep = (ms) => new Promise((resolve) => setTimeout(resolve, ms));

export async function link(server, projectId, ...nodes) {
  const result = await server.api("POST", `/projects/${projectId}/links`, {
    nodes: nodes.map((endpoint) => endpoint.node_id
      ? { node_id: endpoint.node_id, adapter_number: endpoint.adapter_number ?? 0, port_number: endpoint.port_number ?? 0 }
      : { node_id: endpoint.node.node_id, adapter_number: endpoint.adapter_number ?? 0, port_number: endpoint.port_number ?? 0 }),
  });
  if (result.status !== 201) throw new Error(`link failed: ${JSON.stringify(result)}`);
  return result.json;
}

export async function startNode(server, projectId, node) {
  const result = await server.api("POST", `/projects/${projectId}/nodes/${node.node_id}/start`, {});
  if (result.status !== 200) throw new Error(`start ${node.name} failed: ${JSON.stringify(result)}`);
  if (result.json && typeof result.json === "object") Object.assign(node, result.json);
  await waitFor(async () => {
    const status = await server.api("GET", `/projects/${projectId}/nodes`);
    const current = status.json?.find?.((candidate) => candidate.node_id === node.node_id);
    if (current) Object.assign(node, current);
    return status.status === 200 && current?.status === "started";
  }, { timeout: 30000, label: `${node.name} started` });
  return result.json;
}

export async function stopNode(server, projectId, node) {
  const result = await server.api("POST", `/projects/${projectId}/nodes/${node.node_id}/stop`, {});
  if (result.status !== 200) throw new Error(`stop ${node.name} failed: ${JSON.stringify(result)}`);
}

function cleanTelnet(buffer) {
  let output = "";
  for (let i = 0; i < buffer.length; i++) {
    if (buffer[i] !== 255) { output += String.fromCharCode(buffer[i]); continue; }
    const command = buffer[i + 1];
    if (command === 255) { output += String.fromCharCode(255); i++; }
    else if (command === 251 || command === 252 || command === 253 || command === 254) i += 2;
    else i++;
  }
  return output;
}

export async function connectConsole(node, { timeout = 30000 } = {}) {
  const host = node.console_host ?? "127.0.0.1";
  if (!node.console) {
    const kind = node.node_type === "vpcs" ? "vpcs" : node.node_type === "dynamips" ? "dynamips" : node.node_type === "qemu" ? "qemu" : "docker";
    const route = kind === "qemu" ? "qemu" : kind;
    const suffix = kind === "qemu" ? "/console/ws" : "/console/ws";
    const base = new URL(node.api_base);
    const ws = new WebSocket(`${base.protocol === "https:" ? "wss:" : "ws:"}//${base.host}/v3/compute/projects/${node.project_id}/${route}/nodes/${node.node_id}${suffix}`);
    let output = ""; const waiters = [];
    ws.addEventListener("message", (event) => {
      if (typeof event.data === "string") output += event.data;
      else if (event.data instanceof Blob) event.data.text().then((value) => { output += value; for (const wake of waiters.splice(0)) wake(); });
      else output += new TextDecoder().decode(event.data);
      for (const wake of waiters.splice(0)) wake();
    });
    await new Promise((resolve, reject) => {
      const timer = setTimeout(() => reject(new Error(`console WebSocket timed out for ${node.name}`)), timeout);
      ws.addEventListener("open", () => { clearTimeout(timer); resolve(); }, { once: true });
      ws.addEventListener("error", () => { clearTimeout(timer); reject(new Error(`console WebSocket failed for ${node.name}`)); }, { once: true });
    });
    return {
      socket: ws, get output() { return output; },
      async expectRegex(regex, { timeout: waitTimeout = 15000 } = {}) {
        const end = Date.now() + waitTimeout;
        while (!regex.test(output)) {
          if (Date.now() >= end) throw new Error(`timed out waiting for ${regex}; console output:\n${output.slice(-3000)}`);
          await Promise.race([new Promise((resolve) => waiters.push(resolve)), sleep(Math.min(200, end - Date.now()))]);
        }
        return output.match(regex);
      },
      sendLine(line) { ws.send(`${line}\r`); }, close() { ws.close(); },
    };
  }
  const socket = net.createConnection({ host, port: Number(node.console) });
  socket.setEncoding("utf8");
  let output = "";
  const waiters = [];
  socket.on("data", (chunk) => {
    output += cleanTelnet(Buffer.from(chunk, "binary"));
    for (const wake of waiters.splice(0)) wake();
  });
  await new Promise((resolve, reject) => {
    const timer = setTimeout(() => reject(new Error(`console connect timed out for ${node.name}`)), timeout);
    socket.once("connect", () => { clearTimeout(timer); resolve(); });
    socket.once("error", (error) => { clearTimeout(timer); reject(error); });
  });
  const consoleClient = {
    socket,
    get output() { return output; },
    async expectRegex(regex, { timeout: waitTimeout = 15000 } = {}) {
      const end = Date.now() + waitTimeout;
      while (!regex.test(output)) {
        if (Date.now() >= end) throw new Error(`timed out waiting for ${regex}; console output:\n${output.slice(-3000)}`);
        await Promise.race([new Promise((resolve) => waiters.push(resolve)), sleep(Math.min(200, end - Date.now()))]);
      }
      return output.match(regex);
    },
    sendLine(line) { socket.write(`${line}\r`); },
    close() { socket.destroy(); },
  };
  return consoleClient;
}

export async function login(consoleClient, { username, password, emptyPassword = false } = {}) {
  await consoleClient.expectRegex(/(?:login|username)\s*:/i, { timeout: 120000 });
  if (username) consoleClient.sendLine(username);
  if (password !== undefined || emptyPassword) {
    await consoleClient.expectRegex(/password\s*:/i);
    consoleClient.sendLine(password ?? "");
  }
}

export async function scenario(t, name) {
  const server = await startServer();
  const projectId = await newProject(server, name);
  const nodes = [];
  const consoles = [];
  t.after(async () => {
    for (const console of consoles) console.close();
    for (const node of [...nodes].reverse()) {
      try { await stopNode(server, projectId, node); } catch {}
    }
    try { await server.api("DELETE", `/projects/${projectId}`); } finally { await server.stop(); }
  });
  return {
    server, projectId, nodes, consoles,
    async add(template, x = 0, y = 0, nodeName) {
      const node = await addNode(server, projectId, template, x, y, nodeName);
      node.api_base = server.base;
      nodes.push(node);
      return node;
    },
    async console(node, options) { const c = await connectConsole(node, options); consoles.push(c); return c; },
  };
}

export const chrImage = `${process.env.HOME}/.local/share/network-simulator/images/qemu/mikrotik-chr-7.23.8.qcow2`;

export async function chrTemplate(lab, name = "CHR scenario", adapters = 2) {
  const created = await lab.server.api("POST", "/templates", { name, template_type: "qemu", hda_disk_image: chrImage, adapters, memory: 256, cpus: 1, platform: "x86_64", adapter_type: "e1000", hda_disk_interface: "ide", boot_priority: "c", linked_clone: true });
  if (created.status !== 201) throw new Error(`CHR template failed: ${JSON.stringify(created)}`);
  return created.json;
}

// Opens the CHR serial console, logs in as admin with the empty default password and
// returns { run(line, expectRegex, timeout) -> output, close() }.
export async function chrConsole(lab, node) {
  const url = lab.server.base.replace("http", "ws") + `/v3/compute/projects/${lab.projectId}/qemu/nodes/${node.node_id}/console/ws`;
  const ws = new WebSocket(url);
  let output = "";
  ws.addEventListener("message", async (e) => {
    const text = typeof e.data === "string" ? e.data : e.data instanceof Blob ? await e.data.text() : new TextDecoder().decode(e.data);
    output += text;
    if (text.includes("\x1b[6n")) ws.send("\x1b[24;200R");
  });
  await new Promise((resolve, reject) => { ws.addEventListener("open", resolve, { once: true }); ws.addEventListener("error", reject, { once: true }); });
  const sleep = (ms) => new Promise((r) => setTimeout(r, ms));
  const type = async (text) => { for (const c of text) { ws.send(c); await sleep(40); } };
  const expect = async (re, timeout, label) => {
    try { await waitFor(() => re.test(output), { timeout, label }); }
    catch (error) { console.error(`${label}; ws=${ws.readyState}; console tail:`, JSON.stringify(output.slice(-800))); throw error; }
  };
  const nudge = setInterval(() => { try { ws.send("\r"); } catch { /* closed */ } }, 4000);
  try { await expect(/Login:/i, 150000, `${node.name} serial login prompt`); } finally { clearInterval(nudge); }
  await sleep(1500); output = "";
  await type("admin+ct200w\r");
  await expect(/assword:/i, 30000, `${node.name} password prompt`);
  output = ""; await type("\r");
  const prompt = /\[admin@[^\]]*\]\s*>\s*$/;
  await waitFor(async () => {
    if (prompt.test(output)) return true;
    if (/software license\? \[Y\/n\]:\s*$/.test(output)) await type("n\r");
    if (/new password>\s*$/i.test(output)) { output = ""; ws.send("\x03"); await sleep(1500); ws.send("\r"); }
    return false;
  }, { timeout: 90000, label: `${node.name} RouterOS prompt` }).catch((error) => { console.error("CHR prompt; console tail:", JSON.stringify(output.slice(-600))); throw error; });
  return {
    async run(line, re = /\[admin@[^\]]*\]\s*>\s*$/, timeout = 45000) {
      output = ""; await type(line + "\r"); await expect(re, timeout, `${node.name}: ${line}`); return output;
    },
    close() { ws.close(); },
  };
}
