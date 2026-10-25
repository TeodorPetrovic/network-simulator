import test from "node:test";
import assert from "node:assert/strict";
import { existsSync } from "node:fs";
import { scenario, link, startNode, waitFor } from "./lib.mjs";
import { templateByType } from "../harness.mjs";

const image = `${process.env.HOME}/.local/share/network-simulator/images/qemu/mikrotik-chr-7.23.8.qcow2`;
test("MikroTik CHR exchanges traffic with VPCS", async (t) => {
  if (!existsSync(image)) return t.skip("local MikroTik CHR image is absent");
  const lab = await scenario(t, "scenario-mikrotik-basic");
  const created = await lab.server.api("POST", "/templates", {name:"CHR scenario",template_type:"qemu",hda_disk_image:image,adapters:2,memory:512,cpus:1,platform:"x86_64",adapter_type:"e1000",hda_disk_interface:"ide",boot_priority:"c",linked_clone:true});
  assert.equal(created.status,201,JSON.stringify(created));
  const chr = await lab.add(created.json,0,0,"CHR");
  const vpcs = await lab.add(await templateByType(lab.server,"vpcs"),0,0,"VPCS");
  await link(lab.server,lab.projectId,chr,vpcs);
  await Promise.all([startNode(lab.server,lab.projectId,chr),startNode(lab.server,lab.projectId,vpcs)]);
  const pc = await lab.console(vpcs); await pc.expectRegex(/VPCS>/);
  await pc.sendLine("ip 10.1.1.10/24 10.1.1.1"); await pc.expectRegex(/VPCS>/);
  const serialPath=`/v3/compute/projects/${lab.projectId}/qemu/nodes/${chr.node_id}/console/ws`;
  const ws=new WebSocket(lab.server.base.replace("http","ws")+serialPath); let output="";
  ws.addEventListener("message",async e=>{output+=typeof e.data==="string"?e.data:e.data instanceof Blob?await e.data.text():new TextDecoder().decode(e.data);});
  await new Promise((resolve,reject)=>{ws.addEventListener("open",resolve,{once:true});ws.addEventListener("error",reject,{once:true});});
  await waitFor(()=>/MikroTik|Login:|login:|password:|Press ENTER/i.test(output),{timeout:120000,label:"CHR serial login prompt"});
  ws.send("\r"); await waitFor(()=>/Login:/i.test(output),{timeout:30000,label:"CHR login"}); ws.send("admin\r");
  await waitFor(()=>/Password:/i.test(output),{timeout:30000,label:"CHR password prompt"}); ws.send("\r");
  await waitFor(()=>/\[admin@|new password|license|RouterOS/i.test(output),{timeout:30000,label:"CHR RouterOS prompt"});
  for(const line of ["/ip address add address=10.1.1.1/24 interface=ether1","/ping 10.1.1.10 count=3"]) { ws.send(line+"\r"); await waitFor(()=>output.includes("10.1.1.10")||output.includes("[admin@"),{timeout:45000,label:`CHR ${line}`}); }
  await commandPc(pc,"ping 10.1.1.1 -c 2",/84 bytes from 10\.1\.1\.1/,45000);
  ws.close();
});
async function commandPc(c,line,re,timeout){c.sendLine(line);await c.expectRegex(re,{timeout});}
