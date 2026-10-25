import test from "node:test";
import assert from "node:assert/strict";
import { scenario, link, startNode } from "./lib.mjs";
import { templateByType } from "../harness.mjs";

const imageDir = `${process.env.HOME}/.local/share/network-simulator/images/dynamips`;
const ios = "c3620-a3jk8s-mz.122-26c.image";
async function routerTemplate(server) {
  const all = await server.api("GET", "/templates");
  const found = all.json.find((x) => x.template_type === "dynamips" && `${x.image} ${x.platform}`.toLowerCase().includes("c3620"));
  if (found) return found;
  const created = await server.api("POST", "/templates", { name: "Scenario c3620", template_type: "dynamips", platform: "c3620", image: `${imageDir}/${ios}`, ram: 128, nvram: 128, chassis: "3620", slot0: "NM-1FE-TX" });
  if (created.status !== 201) throw new Error(`c3620 template unavailable: ${JSON.stringify(created)}`);
  return created.json;
}
const addVpcs = async (lab, name) => lab.add(await templateByType(lab.server, "vpcs"), 0, 0, name);
async function cmd(c, line, re, timeout = 20000) { c.sendLine(line); await c.expectRegex(re, { timeout }); }
async function setupRouter(lab, template, name) { return lab.add(template, 0, 0, name); }

test("VPCS traffic routes through a c3620", async (t) => {
  if (!await import("node:fs").then(({ existsSync }) => existsSync(`${imageDir}/${ios}`))) return t.skip("c3620 IOS image is absent");
  const lab = await scenario(t, "scenario-vpcs-router");
  const [ta, tr, tb] = await Promise.all([templateByType(lab.server, "vpcs"), routerTemplate(lab.server), templateByType(lab.server, "vpcs")]);
  const a = await lab.add(ta, 0, 0, "A"), r = await setupRouter(lab, tr, "R1"), b = await lab.add(tb, 0, 0, "B");
  await link(lab.server, lab.projectId, a, { node: r, port_number: 0 });
  await link(lab.server, lab.projectId, b, { node: r, port_number: 1 });
  await Promise.all([startNode(lab.server, lab.projectId, a), startNode(lab.server, lab.projectId, r), startNode(lab.server, lab.projectId, b)]);
  const ca = await lab.console(a), cb = await lab.console(b), cr = await lab.console(r);
  await Promise.all([ca.expectRegex(/VPCS>/), cb.expectRegex(/VPCS>/), cr.expectRegex(/Router[>#]/, { timeout: 120000 })]);
  await cmd(ca, "ip 10.0.1.10/24 10.0.1.1", /VPCS>/); await cmd(cb, "ip 10.0.2.10/24 10.0.2.1", /VPCS>/);
  for (const line of ["enable", "configure terminal", "interface FastEthernet0/0", "ip address 10.0.1.1 255.255.255.0", "no shutdown", "interface FastEthernet0/1", "ip address 10.0.2.1 255.255.255.0", "no shutdown", "end"]) await cmd(cr, line, /(?:Router[>#]|\(config[^)]*\)#)/, 120000);
  await cmd(ca, "ping 10.0.2.10 -c 2", /84 bytes from 10\.0\.2\.10/, 30000);
  ca.sendLine("trace 10.0.2.10"); await ca.expectRegex(/10\.0\.1\.1/, { timeout: 30000 });
});

test("two c3620 routers exchange traffic with static routes", async (t) => {
  if (!await import("node:fs").then(({ existsSync }) => existsSync(`${imageDir}/${ios}`))) return t.skip("c3620 IOS image is absent");
  const lab = await scenario(t, "scenario-two-router-static");
  const rt = await routerTemplate(lab.server);
  const [a, r1, r2, b] = await Promise.all([addVpcs(lab,"A"), setupRouter(lab,rt,"R1"), setupRouter(lab,rt,"R2"), addVpcs(lab,"B")]);
  await link(lab.server, lab.projectId, a, {node:r1,port_number:0});
  await link(lab.server, lab.projectId, r1, {node:r2,port_number:0});
  await link(lab.server, lab.projectId, {node:r2,port_number:1}, b);
  await Promise.all([a,r1,r2,b].map(n=>startNode(lab.server,lab.projectId,n)));
  const ca=await lab.console(a), cb=await lab.console(b), c1=await lab.console(r1), c2=await lab.console(r2);
  await Promise.all([ca.expectRegex(/VPCS>/),cb.expectRegex(/VPCS>/),c1.expectRegex(/Router[>#]/,{timeout:120000}),c2.expectRegex(/Router[>#]/,{timeout:120000})]);
  await cmd(ca,"ip 10.0.1.10/24 10.0.1.1",/VPCS>/); await cmd(cb,"ip 10.0.3.10/24 10.0.3.1",/VPCS>/);
  async function config(c, lines) { for (const line of lines) await cmd(c,line,/(?:Router[>#]|\(config[^)]*\)#)/,120000); }
  await config(c1,["enable","configure terminal","interface FastEthernet0/0","ip address 10.0.1.1 255.255.255.0","no shutdown","interface FastEthernet0/1","ip address 10.0.2.1 255.255.255.0","no shutdown","ip route 10.0.3.0 255.255.255.0 10.0.2.2","end"]);
  await config(c2,["enable","configure terminal","interface FastEthernet0/0","ip address 10.0.2.2 255.255.255.0","no shutdown","interface FastEthernet0/1","ip address 10.0.3.1 255.255.255.0","no shutdown","ip route 10.0.1.0 255.255.255.0 10.0.2.1","end"]);
  await cmd(ca,"ping 10.0.3.10 -c 2",/84 bytes from 10\.0\.3\.10/,45000);
  c1.sendLine("traceroute 10.0.3.10"); await c1.expectRegex(/10\.0\.2\.2/,{timeout:45000});
});
