import test from "node:test";
import assert from "node:assert/strict";
import { existsSync } from "node:fs";
import { scenario, link, startNode, chrImage, chrTemplate, chrConsole } from "./lib.mjs";
import { templateByType } from "../harness.mjs";

// VPCS-A -- CHR1 -- CHR2 -- VPCS-B with static routes, end-to-end ping and traceroute.
test("two MikroTik CHR routers route between two VPCS hosts", async (t) => {
  if (!existsSync(chrImage)) return t.skip("local MikroTik CHR image is absent");
  const lab = await scenario(t, "scenario-mikrotik-routing");
  const template = await chrTemplate(lab, "CHR routing", 2);
  const vpcsTemplate = await templateByType(lab.server, "vpcs");
  const [chr1, chr2, a, b] = [await lab.add(template, 0, 0, "CHR1"), await lab.add(template, 200, 0, "CHR2"), await lab.add(vpcsTemplate, 0, 100, "PC-A"), await lab.add(vpcsTemplate, 200, 100, "PC-B")];
  await link(lab.server, lab.projectId, { node: a }, { node: chr1, adapter_number: 0 });
  await link(lab.server, lab.projectId, { node: chr1, adapter_number: 1 }, { node: chr2, adapter_number: 1 });
  await link(lab.server, lab.projectId, { node: chr2, adapter_number: 0 }, { node: b });
  await Promise.all([chr1, chr2, a, b].map((node) => startNode(lab.server, lab.projectId, node)));
  const pa = await lab.console(a), pb = await lab.console(b);
  await pa.expectRegex(/VPCS>/); await pb.expectRegex(/VPCS>/);
  await pa.sendLine("ip 10.0.1.10/24 10.0.1.1"); await pa.expectRegex(/VPCS>/);
  await pb.sendLine("ip 10.0.3.10/24 10.0.3.1"); await pb.expectRegex(/VPCS>/);
  const [r1, r2] = await Promise.all([chrConsole(lab, chr1), chrConsole(lab, chr2)]);
  t.after(() => { r1.close(); r2.close(); });
  await r1.run("/ip address add address=10.0.1.1/24 interface=ether1");
  await r1.run("/ip address add address=10.0.2.1/24 interface=ether2");
  await r1.run("/ip route add dst-address=10.0.3.0/24 gateway=10.0.2.2");
  await r2.run("/ip address add address=10.0.3.1/24 interface=ether1");
  await r2.run("/ip address add address=10.0.2.2/24 interface=ether2");
  await r2.run("/ip route add dst-address=10.0.1.0/24 gateway=10.0.2.1");

  const neighbour = await r1.run("/ping 10.0.2.2 count=3", /sent=3 received=\d+/, 60000);
  if(!/received=[123]/.test(neighbour)){ console.error("DIAG", await r1.run("/interface print"), await r2.run("/interface print"), await r1.run("/ip arp print"), await r1.run("/ip address print")); }
  assert.match(neighbour, /received=[123]/, "CHR1 reaches CHR2 over the point-to-point link");

  await pa.sendLine("ping 10.0.3.10 -c 3");
  await pa.expectRegex(/84 bytes from 10\.0\.3\.10/, { timeout: 60000 });
  await pb.sendLine("ping 10.0.1.10 -c 3");
  await pb.expectRegex(/84 bytes from 10\.0\.1\.10/, { timeout: 60000 });

  await pa.sendLine("trace 10.0.3.10");
  await pa.expectRegex(/10\.0\.1\.1[\s\S]*10\.0\.2\.2[\s\S]*10\.0\.3\.10/, { timeout: 60000 });

  const trace = await r1.run("/tool traceroute 10.0.3.10 count=1", /10\.0\.3\.10[\s\S]*\[admin@[^\]]*\]\s*>\s*$/, 90000);
  assert.match(trace, /10\.0\.2\.2/, "traceroute from CHR1 shows CHR2 as a hop");
});
