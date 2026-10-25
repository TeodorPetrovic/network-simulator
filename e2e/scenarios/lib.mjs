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
