import test from "node:test";
import assert from "node:assert/strict";
import { scenario, link, startNode } from "./lib.mjs";
import { templateByType } from "../harness.mjs";

const addVpcs = async (lab, name) => lab.add(await templateByType(lab.server, "vpcs"), 0, 0, name);
async function command(c, line, regex, timeout = 20000) {
  c.sendLine(line); await c.expectRegex(regex, { timeout });
}

test("two VPCS exchange real traffic and fail across subnets without a gateway", async (t) => {
  const lab = await scenario(t, "scenario-vpcs-pair");
  const a = await addVpcs(lab, "A"), b = await addVpcs(lab, "B");
  await link(lab.server, lab.projectId, a, b);
  await Promise.all([startNode(lab.server, lab.projectId, a), startNode(lab.server, lab.projectId, b)]);
  const ca = await lab.console(a), cb = await lab.console(b);
  await Promise.all([ca.expectRegex(/VPCS>/), cb.expectRegex(/VPCS>/)]);
  await command(ca, "ip 10.10.1.10/24", /VPCS>/);
  await command(cb, "ip 10.10.1.11/24", /VPCS>/);
  await command(ca, "ping 10.10.1.11 -c 1", /84 bytes from 10\.10\.1\.11/);
  await command(cb, "ping 10.10.1.10 -c 1", /84 bytes from 10\.10\.1\.10/);
  await command(cb, "ip 10.10.2.11/24", /VPCS>/);
  ca.sendLine("ping 10.10.2.11 -c 1");
  await ca.expectRegex(/No gateway found|100% packet loss|not reachable|timeout/i);
});

test("three VPCS exchange traffic through a built-in Ethernet switch; stopping a peer removes reachability", async (t) => {
  const lab = await scenario(t, "scenario-vpcs-switch");
  const vpcs = await Promise.all(["A", "B", "C"].map((n) => addVpcs(lab, n)));
  const sw = await lab.add(await templateByType(lab.server, "ethernet_switch"), 0, 0, "Switch");
  for (const [i, node] of vpcs.entries()) await link(lab.server, lab.projectId, node, { node: sw, port_number: i });
  await Promise.all(vpcs.map((n) => startNode(lab.server, lab.projectId, n)));
  const consoles = await Promise.all(vpcs.map((n) => lab.console(n)));
  await Promise.all(consoles.map((c) => c.expectRegex(/VPCS>/)));
  for (let i = 0; i < 3; i++) await command(consoles[i], `ip 10.10.1.${10 + i}/24`, /VPCS>/);
  for (const [from, to] of [[0,1],[0,2],[1,0],[1,2],[2,0],[2,1]]) {
    const ip = `10.10.1.${10 + to}`;
    await command(consoles[from], `ping ${ip} -c 1`, new RegExp(`84 bytes from ${ip.replaceAll(".", "\\.")}`));
  }
  await (await lab.server.api("POST", `/projects/${lab.projectId}/nodes/${vpcs[2].node_id}/stop`, {}));
  consoles[0].sendLine("ping 10.10.1.12 -c 1");
  await consoles[0].expectRegex(/100% packet loss|not reachable|timeout/i);
});
