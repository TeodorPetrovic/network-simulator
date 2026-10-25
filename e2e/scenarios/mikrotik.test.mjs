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
  ws.addEventListener("message",async e=>{const text=typeof e.data==="string"?e.data:e.data instanceof Blob?await e.data.text():new TextDecoder().decode(e.data);output+=text;if(text.includes("\x1b[6n")) ws.send("\x1b[24;200R");});
  await new Promise((resolve,reject)=>{ws.addEventListener("open",resolve,{once:true});ws.addEventListener("error",reject,{once:true});});
  const sleep=(ms)=>new Promise((r)=>setTimeout(r,ms));
  const type=async(text)=>{ for(const c of text){ ws.send(c); await sleep(40); } };
  const expect=async(re,timeout,label)=>{ try { await waitFor(()=>re.test(output),{timeout,label}); } catch (error) { console.error(`${label}; console tail:`, JSON.stringify(output.slice(-800))); throw error; } };
  const run=async(line,re,timeout=45000)=>{ output=""; await type(line+"\r"); await expect(re,timeout,`CHR ${line}`); return output; };
  const nudge=setInterval(()=>{ try{ws.send("\r");}catch{} },4000);
  try { await expect(/Login:/i,150000,"CHR serial login prompt"); } finally { clearInterval(nudge); }
  await sleep(1500); output="";
  await type("admin+ct200w\r"); await expect(/assword:/i,30000,"CHR password prompt");
  output=""; await type("\r");
  const prompt=/\[admin@[^\]]*\]\s*>\s*$/;
  let answeredLicense=false, answeredPassword=false;
  await waitFor(async()=>{
    if(prompt.test(output)) return true;
    if(!answeredLicense && /software license\? \[Y\/n\]:\s*$/.test(output)){ answeredLicense=true; await type("n\r"); }
    if(/new password>\s*$/i.test(output)){ answeredPassword=true; output=""; ws.send("\x03"); await sleep(1500); ws.send("\r"); }
    return false;
  },{timeout:90000,label:"CHR RouterOS prompt"}).catch((error)=>{ console.error("CHR prompt; console tail:",JSON.stringify(output.slice(-600))); throw error; });
  await run("/ip address add address=10.1.1.1/24 interface=ether1",/\[admin@[^\]]*\]\s*>/);
  const printed=await run("/ip address print",/10\.1\.1\.1\/24[\s\S]*\[admin@/);
  assert.match(printed,/ether1/);
  await pc.sendLine("ping 10.1.1.1 -c 2");
  const result=await run("/ping 10.1.1.10 count=3",/sent=3 received=\d+/,60000);
  console.error("CHR ping:",JSON.stringify(result.slice(-500))); assert.match(result,/received=[123]/,"CHR receives replies from the VPCS");
  await commandPc(pc,"ping 10.1.1.1 -c 2",/84 bytes from 10\.1\.1\.1/,45000);
  ws.close();
});
async function commandPc(c,line,re,timeout){c.sendLine(line);await c.expectRegex(re,{timeout});}
