import test from "node:test";
import assert from "node:assert/strict";
import { execFileSync } from "node:child_process";
import { scenario, link, startNode } from "./lib.mjs";
import { templateByType } from "../harness.mjs";

function dockerExec(id, command) { return execFileSync("docker",["exec",id,"sh","-lc",command],{encoding:"utf8",timeout:30000}); }
async function docker(lab,name,adapters=1){const r=await lab.server.api("POST",`/projects/${lab.projectId}/nodes`,{name,node_type:"docker",image:"alpine:latest",start_command:"/bin/sh -c 'sleep 3600'",adapters});assert.equal(r.status,201,JSON.stringify(r));r.json.api_base=lab.server.base;lab.nodes.push(r.json);return r.json;}
test("Alpine nodes use real container networking and can exchange traffic with VPCS",async(t)=>{
  try{execFileSync("docker",["image","inspect","alpine:latest"],{stdio:"ignore"});}catch{return t.skip("Docker daemon or alpine:latest image is unavailable");}
  const lab=await scenario(t,"scenario-docker");
  const [a,b]=await Promise.all([docker(lab,"Alpine A"),docker(lab,"Alpine B")]);
  const v=await lab.add(await templateByType(lab.server,"vpcs"),0,0,"VPCS");
  const sw=await lab.server.api("POST",`/projects/${lab.projectId}/nodes`,{name:"Switch",node_type:"ethernet_switch"});assert.equal(sw.status,201);lab.nodes.push(sw.json);
  await link(lab.server,lab.projectId,{node:a,adapter_number:0},{node:sw.json,port_number:0});
  await link(lab.server,lab.projectId,{node:b,adapter_number:0},{node:sw.json,port_number:1});
  await link(lab.server,lab.projectId,v,{node:sw.json,port_number:2});
  await Promise.all([startNode(lab.server,lab.projectId,a),startNode(lab.server,lab.projectId,b),startNode(lab.server,lab.projectId,v)]);
  const info=dockerExec(a.container_id,"ip addr");assert.match(info,/eth/);
  dockerExec(a.container_id,"ip link set eth0 up && ip addr add 10.9.0.1/24 dev eth0");
  dockerExec(b.container_id,"ip link set eth0 up && ip addr add 10.9.0.2/24 dev eth0");
  assert.match(dockerExec(a.container_id,"ping -c 2 -W 2 10.9.0.2"),/2 packets transmitted, 2 packets received/);
  const pc=await lab.console(v);await pc.expectRegex(/VPCS>/);pc.sendLine("ip 10.9.0.3/24");await pc.expectRegex(/VPCS>/);
  assert.match(dockerExec(b.container_id,"ping -c 2 -W 2 10.9.0.3"),/2 packets transmitted, 2 packets received/);
});
