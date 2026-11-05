// NAT node gives a VPCS guest DHCP, DNS and internet access through the host's
// libvirt default network (virbr0). Skipped when the host has no such bridge.
import test from "node:test";
import assert from "node:assert/strict";
import { existsSync } from "node:fs";
import { startServer, newProject, templateByType, addNode, waitFor } from "./harness.mjs";

const hasNat = existsSync("/sys/class/net/virbr0");

test("VPCS behind a NAT node gets DHCP and reaches the internet", { skip: !hasNat && "no virbr0 on host" }, async (t) => {
  const server = await startServer();
  t.after(() => server.stop());
  const pid = await newProject(server, "nat-e2e");
  const nat = await addNode(server, pid, await templateByType(server, "nat"), 0, 0);
  const pc = await addNode(server, pid, await templateByType(server, "vpcs"), 200, 0, "PC1");
  const link = await server.api("POST", `/projects/${pid}/links`, {
    nodes: [
      { node_id: pc.node_id, adapter_number: 0, port_number: 0 },
      { node_id: nat.node_id, adapter_number: 0, port_number: 0 },
    ],
  });
  assert.equal(link.status, 201, JSON.stringify(link.json));
  for (const id of [nat.node_id, pc.node_id]) {
    assert.equal((await server.api("POST", `/projects/${pid}/nodes/${id}/start`, {})).status, 200);
  }
  const ws = new WebSocket(server.base.replace("http", "ws") + `/v3/projects/${pid}/nodes/${pc.node_id}/console/ws`);
  ws.binaryType = "arraybuffer";
  let out = "";
  ws.onmessage = (e) => { out += typeof e.data === "string" ? e.data : Buffer.from(e.data).toString(); };
  await new Promise((ok, fail) => { ws.onopen = ok; ws.onerror = fail; });
  t.after(() => ws.close());
  const run = async (command, pattern, timeout) => {
    const from = out.length;
    ws.send(command + "\r");
    await waitFor(() => pattern.test(out.slice(from)), { timeout, label: command });
  };
  ws.send("\r");
  await waitFor(() => /VPCS>/.test(out), { timeout: 15000, label: "VPCS prompt" });
  await run("dhcp", /IP 192\.168\.\d+\.\d+\/\d+ GW/, 20000);
  await run("ping 8.8.8.8 -c 2", /bytes from 8\.8\.8\.8 icmp_seq=2/, 20000);

  // Resetting a running link rebuilds its plumbing; traffic must still flow afterwards.
  const reset = await server.api("POST", `/projects/${pid}/links/${link.json.link_id}/reset`, {});
  assert.equal(reset.status, 204, JSON.stringify(reset.json));
  await run("ping 8.8.8.8 -c 2", /bytes from 8\.8\.8\.8 icmp_seq=2/, 30000);
  assert.equal((await server.api("POST", "/reload", {})).status, 204);
});
