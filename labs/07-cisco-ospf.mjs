import { bootIos, telnet, sleep } from "./lib.mjs";

export const id = "07-cisco-ospf";
export const title = "Cisco IOS routers with OSPF";
export const summary = "Three Dynamips Cisco 3725 routers in a line run OSPF area 0 between two VPCS LANs.";
export const appliances = ["cisco-3725.gns3a"];
export const readme = `
PC1 10.1.1.10/24 -- R1 -- R2 -- R3 -- PC2 10.3.3.10/24. IOS images come from the global image library (c3725).
Open each router's console; OSPF is configured by \`bun scripts/labs.mjs configure 07\` (or deploy --start).`;

const routers = {
  r1: { id: "1.1.1.1", ifs: [["fa0/0", "10.1.1.1 255.255.255.0"], ["fa0/1", "10.0.12.1 255.255.255.252"]] },
  r2: { id: "2.2.2.2", ifs: [["fa0/0", "10.0.12.2 255.255.255.252"], ["fa0/1", "10.0.23.1 255.255.255.252"]] },
  r3: { id: "3.3.3.3", ifs: [["fa0/0", "10.0.23.2 255.255.255.252"], ["fa0/1", "10.3.3.1 255.255.255.0"]] },
};

const iosConfig = (name, r) => [
  `hostname ${name}`, "!", "no ip domain-lookup", "!",
  "interface Loopback0", ` ip address ${r.id} 255.255.255.255`, "!",
  ...r.ifs.flatMap(([ifc, addr]) => [`interface ${ifc}`, ` ip address ${addr}`, " no shutdown", "!"]),
  "router ospf 1", ` router-id ${r.id}`, " network 10.0.0.0 0.255.255.255 area 0", ` network ${r.id} 0.0.0.0 area 0`, "!", "end", "",
].join("\n");

export async function build(lab) {
  await lab.vpcs("pc1", { name: "PC1", x: -400, y: 0 });
  await lab.configure("pc1", { startup_script: "ip 10.1.1.10/24 10.1.1.1" });
  for (const [i, [key, r]] of Object.entries(routers).entries()) {
    await lab.device(key, "cisco-3725.gns3a", { name: key.toUpperCase(), x: -200 + i * 200, y: 0, set: { startup_config: iosConfig(key.toUpperCase(), r) } });
  }
  await lab.vpcs("pc2", { name: "PC2", x: 400, y: 0 });
  await lab.configure("pc2", { startup_script: "ip 10.3.3.10/24 10.3.3.1" });
  await lab.link(["pc1", 0], ["r1", "0/0"]);
  await lab.link(["r1", "0/1"], ["r2", "0/0"]);
  await lab.link(["r2", "0/1"], ["r3", "0/0"]);
  await lab.link(["r3", "0/1"], ["pc2", 0]);
}

export async function verify(lab, check) {
  await lab.startAll();
  const r2 = await bootIos(lab.node("r2"));
  let neighbors = "";
  for (let i = 0; i < 30; i += 1) { neighbors = await r2.run("show ip ospf neighbor"); if ((neighbors.match(/FULL/g) ?? []).length >= 2) break; await sleep(5000); }
  check("R2 has two FULL OSPF neighbours", (neighbors.match(/FULL/g) ?? []).length >= 2);
  const r1 = await bootIos(lab.node("r1"));
  let routes = "";
  for (let i = 0; i < 12 && !/10\.3\.3\.0/.test(routes); i += 1) { routes = await r1.run("show ip route ospf"); if (!/10\.3\.3\.0/.test(routes)) await sleep(5000); }
  r1.close();
  check("R1 learns the R3 LAN via OSPF", /10\.3\.3\.0/.test(routes));
  r2.close();
  const pc = await telnet(lab.node("pc1"));
  let out = "";
  for (let i = 0; i < 6 && !/ttl=61/.test(out); i += 1) { pc.send("\r"); pc.clear(); pc.send("ping 10.3.3.10 -c 3\r"); await sleep(8000); out = pc.output; }
  pc.close();
  check("PC1 reaches PC2 across three routers (TTL 61)", /ttl=61/.test(out));
}
