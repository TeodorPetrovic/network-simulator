import test from "node:test";
import assert from "node:assert/strict";
import { existsSync } from "node:fs";
import { scenario, link, startNode, chrImage, chrTemplate, chrConsole } from "./lib.mjs";
import { templateByType } from "../harness.mjs";

function uptimeSeconds(text) {
  const match = text.match(/uptime:\s*([^\r\n]+)/i)?.[1]?.trim();
  assert.ok(match, "CHR resource output includes uptime");
  const units = [...match.matchAll(/(\d+)([wdhms])/g)];
  if (units.length) return units.reduce((sum, [, value, unit]) => sum + Number(value) * ({ w: 604800, d: 86400, h: 3600, m: 60, s: 1 })[unit], 0);
  const clock = match.match(/^(?:(\d+):)?(\d+):(\d+)$/);
  assert.ok(clock, `unrecognized CHR uptime: ${match}`);
  return Number(clock[1] ?? 0) * 3600 + Number(clock[2]) * 60 + Number(clock[3]);
}

test("MikroTik CHR accepts and removes links while running without rebooting", async (t) => {
  if (!existsSync(chrImage)) return t.skip("local MikroTik CHR image is absent");
  const lab = await scenario(t, "scenario-mikrotik-hotlink");
  const template = await chrTemplate(lab, "CHR hot link", 2);
  const chr = await lab.add(template, 0, 0, "CHR");
  const vpcsTemplate = await templateByType(lab.server, "vpcs");
  const [pc1, pc2] = await Promise.all([
    lab.add(vpcsTemplate, 0, 100, "PC1"),
    lab.add(vpcsTemplate, 200, 100, "PC2"),
  ]);
  await startNode(lab.server, lab.projectId, chr);
  const router = await chrConsole(lab, chr);
  t.after(() => router.close());
  const initial = await router.run("/system resource print", /uptime:\s*([^\r\n]+)/i, 30000);
  const initialUptime = uptimeSeconds(initial);

  await Promise.all([startNode(lab.server, lab.projectId, pc1), startNode(lab.server, lab.projectId, pc2)]);
  const c1 = await lab.console(pc1), c2 = await lab.console(pc2);
  t.after(() => { c1.close(); c2.close(); });
  await Promise.all([c1.expectRegex(/VPCS>/), c2.expectRegex(/VPCS>/)]);
  await link(lab.server, lab.projectId, { node: chr }, { node: pc1 });
  await c1.sendLine("ip 10.1.1.10/24 10.1.1.1"); await c1.expectRegex(/VPCS>/);
  await router.run("/ip address add address=10.1.1.1/24 interface=ether1");
  await c1.sendLine("ping 10.1.1.1 -c 2");
  await c1.expectRegex(/84 bytes from 10\.1\.1\.1/, { timeout: 45000 });
  await router.run("/ping 10.1.1.10 count=3", /sent=3 received=[123]/, 60000);
  const stillRunning = await router.run("/system resource print", /uptime:\s*([^\r\n]+)/i);
  assert.match(stillRunning, /uptime:/i, "same open serial console still answers after attach");
  assert.ok(uptimeSeconds(stillRunning) >= initialUptime, "uptime did not reset after the first hot link");

  const links = await lab.server.api("GET", `/projects/${lab.projectId}/links`);
  assert.equal(links.status, 200, JSON.stringify(links));
  const deleted = await lab.server.api("DELETE", `/projects/${lab.projectId}/links/${links.json[0].link_id}`);
  assert.equal(deleted.status, 204, JSON.stringify(deleted));
  c1.sendLine("ping 10.1.1.1 -c 2");
  await c1.expectRegex(/icmp_seq=2 timeout|100% packet loss|unreachable/i, { timeout: 15000 });

  await link(lab.server, lab.projectId, { node: chr }, { node: pc1 });
  await c1.sendLine("ping 10.1.1.1 -c 2");
  await c1.expectRegex(/84 bytes from 10\.1\.1\.1/, { timeout: 45000 });
  await link(lab.server, lab.projectId, { node: chr, adapter_number: 1 }, { node: pc2 });
  await c2.sendLine("ip 10.1.2.10/24 10.1.2.1"); await c2.expectRegex(/VPCS>/);
  await router.run("/ip address add address=10.1.2.1/24 interface=ether2");
  await c2.sendLine("ping 10.1.2.1 -c 2");
  await c2.expectRegex(/84 bytes from 10\.1\.2\.1/, { timeout: 45000 });
  const final = await router.run("/system resource print", /uptime:\s*([^\r\n]+)/i);
  assert.ok(uptimeSeconds(final) > initialUptime, "uptime continues increasing while links are attached");
});
