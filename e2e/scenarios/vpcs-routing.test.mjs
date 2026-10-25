import test from "node:test";
import assert from "node:assert/strict";
import { scenario, link, startNode } from "./lib.mjs";
import { templateByType } from "../harness.mjs";

const imageDir = `${process.env.HOME}/.local/share/network-simulator/images/dynamips`;
const ios = "c3620-a3jk8s-mz.122-26c.image";
async function routerTemplate(server) {
  const all = await server.api("GET", "/templates");
  const found = all.json.find((x) => x.template_type === "dynamips" && x.platform === "c3600" && x.chassis === "3620" && x.slot0 === "NM-1FE-TX" && x.slot1 === "NM-1FE-TX");
  if (found) return found;
  const created = await server.api("POST", "/templates", { name: "Scenario c3620", template_type: "dynamips", platform: "c3600", image: `${imageDir}/${ios}`, ram: 128, nvram: 128, chassis: "3620", slot0: "NM-1FE-TX", slot1: "NM-1FE-TX", adapters: 2 });
  if (created.status !== 201) throw new Error(`c3620 template unavailable: ${JSON.stringify(created)}`);
  return created.json;
}
const addVpcs = async (lab, name) => lab.add(await templateByType(lab.server, "vpcs"), 0, 0, name);
async function cmd(c, line, re, timeout = 20000) {
  const start = c.output.length;
  const deadline = Date.now() + timeout;
  c.sendLine(line);
  while (Date.now() < deadline) {
    if (re.test(c.output.slice(start))) return;
    await new Promise((resolve) => setTimeout(resolve, 100));
  }
  throw new Error(`timed out waiting for ${re}; console output:\n${c.output.slice(start, start + 3000)}`);
}
async function setupRouter(lab, template, name) { return lab.add(template, 0, 0, name); }
async function routerPrompt(consoleClient) {
  const output = await consoleClient.expectRegex(/Router[>#]|Would you like to enter the initial configuration dialog\? \[yes\/no\]:/,{timeout:120000});
  if (/initial configuration dialog/.test(output[0])) {
    await cmd(consoleClient, "no", /Press RETURN to get started/, 120000);
    await cmd(consoleClient, "", /Router[>#]/, 120000);
  }
}
async function diagnosticCommand(client, line) {
  const start = client.output.length;
  client.sendLine(line);
  const deadline = Date.now() + 8000;
  while (Date.now() < deadline) {
    const response = client.output.slice(start);
    if (/Router[>#]\s*$/.test(response)) return response;
    await new Promise((resolve) => setTimeout(resolve, 100));
  }
  return client.output.slice(start);
}
async function diagnosticVpcs(client, line) {
  const start = client.output.length;
  client.sendLine(line);
  const deadline = Date.now() + 8000;
  while (Date.now() < deadline) {
    const response = client.output.slice(start);
    if (/VPCS>\s*$/.test(response)) return response;
    await new Promise((resolve) => setTimeout(resolve, 100));
  }
  return client.output.slice(start);
}

test("VPCS traffic routes through a c3620", async (t) => {
  if (!await import("node:fs").then(({ existsSync }) => existsSync(`${imageDir}/${ios}`))) return t.skip("c3620 IOS image is absent");
  const lab = await scenario(t, "scenario-vpcs-router");
  const [ta, tr, tb] = await Promise.all([templateByType(lab.server, "vpcs"), routerTemplate(lab.server), templateByType(lab.server, "vpcs")]);
  const a = await lab.add(ta, 0, 0, "A"), r = await setupRouter(lab, tr, "R1"), b = await lab.add(tb, 0, 0, "B");
  await link(lab.server, lab.projectId, a, { node: r, adapter_number: 0, port_number: 0 });
  await link(lab.server, lab.projectId, b, { node: r, adapter_number: 1, port_number: 0 });
  await Promise.all([startNode(lab.server, lab.projectId, a), startNode(lab.server, lab.projectId, r), startNode(lab.server, lab.projectId, b)]);
  const ca = await lab.console(a), cb = await lab.console(b), cr = await lab.console(r);
  await Promise.all([ca.expectRegex(/VPCS>/), cb.expectRegex(/VPCS>/), routerPrompt(cr)]);
  await cmd(ca, "ip 10.0.1.10/24 10.0.1.1", /VPCS>/); await cmd(cb, "ip 10.0.2.10/24 10.0.2.1", /VPCS>/);
  for (const line of ["enable", "configure terminal", "interface FastEthernet0/0", "ip address 10.0.1.1 255.255.255.0", "no shutdown", "interface FastEthernet1/0", "ip address 10.0.2.1 255.255.255.0", "no shutdown", "end"]) await cmd(cr, line, /(?:Router[>#]|\(config[^)]*\)#)/, 120000);
  await cmd(ca, "ping 10.0.2.10 -c 2", /84 bytes from 10\.0\.2\.10/, 30000);
  ca.sendLine("trace 10.0.2.10"); await ca.expectRegex(/10\.0\.1\.1/, { timeout: 30000 });
});

test("two c3620 routers exchange traffic with static routes", async (t) => {
  if (!await import("node:fs").then(({ existsSync }) => existsSync(`${imageDir}/${ios}`))) return t.skip("c3620 IOS image is absent");
  const lab = await scenario(t, "scenario-two-router-static");
  const rt = await routerTemplate(lab.server);
  const [a, r1, r2, b] = await Promise.all([addVpcs(lab,"A"), setupRouter(lab,rt,"R1"), setupRouter(lab,rt,"R2"), addVpcs(lab,"B")]);
  await link(lab.server, lab.projectId, {node_id:a.node_id,adapter_number:0,port_number:0}, {node:r1,adapter_number:0,port_number:0});
  await link(lab.server, lab.projectId, {node_id:r1.node_id,adapter_number:1,port_number:0}, {node:r2,adapter_number:0,port_number:0});
  await link(lab.server, lab.projectId, {node:r2,adapter_number:1,port_number:0}, {node_id:b.node_id,adapter_number:0,port_number:0});
  await Promise.all([startNode(lab.server,lab.projectId,a),startNode(lab.server,lab.projectId,b)]);
  const ca=await lab.console(a), cb=await lab.console(b);
  await Promise.all([ca.expectRegex(/VPCS>/),cb.expectRegex(/VPCS>/)]);
  await cmd(ca,"ip 10.0.1.10/24 10.0.1.1",/VPCS>/); await cmd(cb,"ip 10.0.3.10/24 10.0.3.1",/VPCS>/);
  await startNode(lab.server,lab.projectId,r1);
  const c1=await lab.console(r1);
  await routerPrompt(c1);
  await startNode(lab.server,lab.projectId,r2);
  const c2=await lab.console(r2);
  await routerPrompt(c2);
  async function config(c, lines) { for (const line of lines) await cmd(c,line,/(?:Router[>#]|\(config[^)]*\)#)/,120000); }
  await config(c1,["enable","configure terminal","interface FastEthernet0/0","ip address 10.0.1.1 255.255.255.0","no shutdown","interface FastEthernet1/0","ip address 10.0.2.1 255.255.255.0","no shutdown","ip route 10.0.3.0 255.255.255.0 10.0.2.2","end"]);
  await config(c2,["enable","configure terminal","interface FastEthernet0/0","ip address 10.0.2.2 255.255.255.0","no shutdown","interface FastEthernet1/0","ip address 10.0.3.1 255.255.255.0","no shutdown","ip route 10.0.1.0 255.255.255.0 10.0.2.1","end"]);
  try {
    await cmd(ca,"ping 10.0.3.10 -c 2",/84 bytes from 10\.0\.3\.10/,45000);
  } catch (error) {
    const diagnostics = [];
    for (const [client, command] of [[c1,"show ip interface brief"],[c1,"show ip route"],[c1,"show ip arp"],[c1,"ping 10.0.2.2"],[c1,"ping 10.0.3.10"],[c1,"traceroute 10.0.3.10"],[c2,"show ip interface brief"],[c2,"show ip route"],[c2,"show ip arp"],[c2,"ping 10.0.3.10"],[c2,"ping 10.0.1.10"]]) {
      diagnostics.push(`${command}: ${await diagnosticCommand(client, command)}`);
    }
    diagnostics.push(`VPCS A: ${await diagnosticVpcs(ca,"show ip")}`);
    diagnostics.push(`VPCS B: ${await diagnosticVpcs(cb,"show ip")}`);
    throw new Error(`${error.message}\n${diagnostics.join("\n")}`);
  }
  c1.sendLine("traceroute 10.0.3.10"); await c1.expectRegex(/10\.0\.2\.2[\s\S]*10\.0\.3\.10/,{timeout:60000});
});
