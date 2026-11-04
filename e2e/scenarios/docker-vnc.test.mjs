import test from "node:test";
import assert from "node:assert/strict";
import { scenario, startNode } from "./lib.mjs";
test("Docker node bridges to a VNC server running inside the container", async (t) => {
  const lab = await scenario(t, "dbg-vnc");
  const r = await lab.server.api("POST", `/projects/${lab.projectId}/nodes`, {name:"vncbox",node_type:"docker",image:"alpine:latest",
    start_command:`/bin/sh -c 'while true; do printf "RFB 003.008\\n" | nc -l -p 5900; done'`, adapters:1, console_type:"vnc", console_vnc_port:5900});
  assert.equal(r.status, 201, JSON.stringify(r));
  const node = r.json; node.api_base = lab.server.base; lab.nodes.push(node);
  assert.equal(node.console_vnc_port, 5900);
  await startNode(lab.server, lab.projectId, node);
  await new Promise(r=>setTimeout(r,1500));
  const url = lab.server.base.replace("http","ws") + `/v3/compute/projects/${lab.projectId}/docker/nodes/${node.node_id}/vnc/ws`;
  const got = await new Promise((resolve,reject)=>{ const ws=new WebSocket(url); const t=setTimeout(()=>reject(new Error("timeout")),8000);
    ws.addEventListener("message", async e=>{ const d = typeof e.data==="string"?e.data:e.data instanceof Blob?await e.data.text():new TextDecoder().decode(e.data); clearTimeout(t); ws.close(); resolve(d); });
    ws.addEventListener("error", ()=>reject(new Error("ws error"))); });
  
  assert.match(got, /^RFB 003\.008/);
});
