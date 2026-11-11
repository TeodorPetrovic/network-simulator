import { httpLoop, sleep } from "./lib.mjs";

export const id = "04-webterm-web";
export const title = "Webterm browser, web servers and Internet via NAT";
export const summary = "A Firefox 'webterm' desktop (VNC console) browses internal web servers and the Internet through a NAT node.";
export const appliances = ["webterm.gns3a", "alpine-docker.gns3a"];
export const readme = `
Built from the registry appliances \`webterm\` and \`alpine-docker\`. Open the Webterm node's console (VNC): Firefox starts
automatically. Browse to http://10.4.0.80 (web1) or http://10.4.0.81 (web2). eth1 on the webterm is bridged to the host's
libvirt NAT network (192.168.122.250/24, gateway .1) so https://example.com works too.`;

export async function build(lab) {
  await lab.switch("lan", { name: "LAN-SW", x: 0, y: 0 });
  await lab.nat("nat", { x: 0, y: -200 });
  await lab.device("term", "webterm.gns3a", { name: "Webterm", x: -250, y: 0, set: { adapters: 2 }, net: {
    eth0: { ip: "10.4.0.10/24" },
    eth1: { ip: "192.168.122.250/24", gw: "192.168.122.1", dns: "192.168.122.1" },
  } });
  for (const n of [0, 1]) {
    await lab.device(`web${n + 1}`, "alpine-docker.gns3a", { name: `Web${n + 1}`, x: 250, y: n ? 100 : -100,
      set: { start_command: httpLoop(`<h1>web${n} on the lab network</h1>`) }, net: { eth0: { ip: `10.4.0.${80 + n}/24` } } });
  }
  await lab.link(["term", 0], ["lan", 0]);
  await lab.link(["web1", 0], ["lan", 1]);
  await lab.link(["web2", 0], ["lan", 2]);
  await lab.link(["term", 1], ["nat", 0]);
}

async function rfbBanner(lab, node) {
  const url = `${lab.cli.base.replace("http", "ws")}/v3/compute/projects/${lab.projectId}/docker/nodes/${node.node_id}/vnc/ws`;
  return new Promise((resolve, reject) => {
    const ws = new WebSocket(url); const timer = setTimeout(() => reject(new Error("VNC websocket timeout")), 15000);
    ws.addEventListener("message", async (event) => { const data = typeof event.data === "string" ? event.data : event.data instanceof Blob ? await event.data.text() : new TextDecoder().decode(event.data); clearTimeout(timer); ws.close(); resolve(data); });
    ws.addEventListener("error", () => reject(new Error("VNC websocket error")));
  });
}

export async function verify(lab, check) {
  await lab.startAll();
  await lab.expectExec("term", "curl -s -m 3 http://10.4.0.80", /web0 on the lab network/, { timeout: 90000 });
  await lab.expectExec("term", "curl -s -m 3 http://10.4.0.81", /web1 on the lab network/, { timeout: 30000 });
  check("webterm reaches both internal web servers", true);
  await sleep(2000);
  check("webterm VNC console serves an RFB handshake", /^RFB 00\d\.00\d/.test(await rfbBanner(lab, lab.node("term"))));
  check("Firefox is running in the webterm", /firefox/.test(lab.exec("term", "ps aux")));
  try {
    await lab.expectExec("term", "curl -s -m 5 -o /dev/null -w '%{http_code}' http://example.com", /^200/, { timeout: 60000 });
    check("webterm reaches the Internet through NAT", true);
  } catch (error) { check("webterm reaches the Internet through NAT", false); console.log(`    ${error.message.split("\n")[0]}`); }
}
