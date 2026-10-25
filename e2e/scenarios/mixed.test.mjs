import test from "node:test";
import assert from "node:assert/strict";
import { existsSync } from "node:fs";
import { execFileSync } from "node:child_process";
import { scenario, link, startNode } from "./lib.mjs";
import { templateByType } from "../harness.mjs";

const imageDir=`${process.env.HOME}/.local/share/network-simulator/images`;
test("VPCS, Docker, CHR and c3620 share a live switched Ethernet segment",async(t)=>{
  if(!existsSync(`${imageDir}/qemu/mikrotik-chr-7.23.8.qcow2`)||!existsSync(`${imageDir}/dynamips/c3620-a3jk8s-mz.122-26c.image`))return t.skip("CHR qcow2 or c3620 IOS image is absent");
  try{execFileSync("docker",["image","inspect","alpine:latest"],{stdio:"ignore"});}catch{return t.skip("Docker daemon or alpine:latest image is unavailable");}
  const lab=await scenario(t,"scenario-mixed-vendors");
  const q=await lab.server.api("POST","/templates",{name:"CHR mixed",template_type:"qemu",hda_disk_image:`${imageDir}/qemu/mikrotik-chr-7.23.8.qcow2`,adapters:2,memory:512,cpus:1,platform:"x86_64",adapter_type:"e1000",hda_disk_interface:"ide",boot_priority:"c",linked_clone:true});assert.equal(q.status,201,JSON.stringify(q));
  const rt=(await lab.server.api("GET","/templates")).json.find(x=>x.template_type==="dynamips"&&String(x.image).includes("c3620"));assert.ok(rt,"c3620 runtime template is required");
  const v=await lab.add(await templateByType(lab.server,"vpcs"),0,0,"PC");
  const chr=await lab.add(q.json,0,0,"CHR"); const r=await lab.add(rt,0,0,"Router");
  const dr=await lab.server.api("POST",`/projects/${lab.projectId}/nodes`,{name:"Alpine",node_type:"docker",image:"alpine:latest",start_command:"/bin/sh -c 'sleep 3600'",adapters:1});assert.equal(dr.status,201);dr.json.api_base=lab.server.base;lab.nodes.push(dr.json);
  const sw=await lab.server.api("POST",`/projects/${lab.projectId}/nodes`,{name:"Switch",node_type:"ethernet_switch"});assert.equal(sw.status,201);lab.nodes.push(sw.json);
  for(const [i,node] of [v,chr,r,dr.json].entries())await link(lab.server,lab.projectId,node.node_type==="docker"?{node,adapter_number:0}:{node},{node:sw.json,port_number:i});
  await Promise.all([v,chr,r,dr.json].map(n=>startNode(lab.server,lab.projectId,n)));
  const pc=await lab.console(v);await pc.expectRegex(/VPCS>/);pc.sendLine("ip 10.20.0.10/24");await pc.expectRegex(/VPCS>/);
  const docker=execFileSync("docker",["exec",dr.json.container_id,"sh","-lc","ip addr"],{encoding:"utf8"});assert.match(docker,/eth0/);
  execFileSync("docker",["exec",dr.json.container_id,"sh","-lc","ip link set eth0 up && ip addr add 10.20.0.20/24 dev eth0"]);
  const ping=execFileSync("docker",["exec",dr.json.container_id,"ping","-c","2","-W","2","10.20.0.10"],{encoding:"utf8"});assert.match(ping,/2 packets transmitted, 2 received/);
});
