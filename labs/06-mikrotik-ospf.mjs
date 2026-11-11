import { routerOs, telnet, sleep } from "./lib.mjs";

export const id = "06-mikrotik-ospf";
export const title = "MikroTik OSPF triangle with failover";
export const summary = "Three RouterOS CHRs run OSPF area 0 in a triangle; two LANs keep talking when the direct link is cut.";
export const appliances = ["mikrotik-chr.gns3a"];
export const readme = `
PC1 (10.1.1.10) and PC2 (10.3.3.10) talk across R1-R3 directly (TTL 62). Delete the R1-R3 link in the UI and the
route moves through R2 after the OSPF dead interval (TTL 61). The packaged project ships each CHR disk with its configuration already applied.`;

const plan = {
  r1: { rid: "1.1.1.1", addrs: [["ether1", "10.1.1.1/24"], ["ether2", "10.0.12.1/30"], ["ether3", "10.0.13.1/30"]], nets: ["10.1.1.0/24:passive", "10.0.12.0/30", "10.0.13.0/30"] },
  r2: { rid: "2.2.2.2", addrs: [["ether1", "10.0.12.2/30"], ["ether2", "10.0.23.1/30"]], nets: ["10.0.12.0/30", "10.0.23.0/30"] },
  r3: { rid: "3.3.3.3", addrs: [["ether1", "10.0.13.2/30"], ["ether2", "10.0.23.2/30"], ["ether3", "10.3.3.1/24"]], nets: ["10.3.3.0/24:passive", "10.0.13.0/30", "10.0.23.0/30"] },
};

export async function build(lab) {
  await lab.device("r1", "mikrotik-chr.gns3a", { name: "R1", image: "mikrotik-chr-7.23.8", set: { adapters: 3 }, x: -200, y: 0 });
  await lab.device("r2", "mikrotik-chr.gns3a", { name: "R2", image: "mikrotik-chr-7.23.8", set: { adapters: 3 }, x: 0, y: 150 });
  await lab.device("r3", "mikrotik-chr.gns3a", { name: "R3", image: "mikrotik-chr-7.23.8", set: { adapters: 3 }, x: 200, y: 0 });
  await lab.vpcs("pc1", { name: "PC1", x: -350, y: 0 });
  await lab.vpcs("pc2", { name: "PC2", x: 350, y: 0 });
  await lab.configure("pc1", { startup_script: "ip 10.1.1.10/24 10.1.1.1" });
  await lab.configure("pc2", { startup_script: "ip 10.3.3.10/24 10.3.3.1" });
  await lab.link(["pc1", 0], ["r1", 0]);
  await lab.link(["r1", 1], ["r2", 0]);
  await lab.link(["r1", 2], ["r3", 0]);
  await lab.link(["r2", 1], ["r3", 1]);
  await lab.link(["r3", 2], ["pc2", 0]);
}

async function setupVpcs(lab, key, ip) {
  const con = await telnet(lab.node(key));
  await con.expect(/VPCS>/, { timeout: 30000 });
  con.clear(); con.send(`${ip}\r`); await con.expect(/VPCS>/);
  return con;
}

export async function configure(lab) {
  await Promise.all(Object.entries(plan).map(async ([key, p]) => {
    const r = await routerOs(lab, lab.node(key));
    try {
      await r.run(`/system identity set name=${key.toUpperCase()}`);
      for (const [iface, addr] of p.addrs) await r.run(`/ip address add address=${addr} interface=${iface}`);
      await r.run(`/interface bridge add name=lo0`);
      await r.run(`/ip address add address=${p.rid}/32 interface=lo0`);
      await r.run(`/routing ospf instance add name=o1 version=2 router-id=${p.rid}`);
      await r.run(`/routing ospf area add name=bb area-id=0.0.0.0 instance=o1`);
      for (const net of [`${p.rid}/32`, ...p.nets]) {
        const [n, flag] = net.split(":");
        await r.run(`/routing ospf interface-template add area=bb networks=${n}${flag ? " passive" : ""}`);
      }
    } finally { r.close(); }
  }));
}

async function ping(lab, from, to) {
  const con = await telnet(lab.node(from));
  try {
    con.send("\r"); await con.expect(/VPCS>/); con.clear();
    con.send(`ping ${to} -c 3\r`);
    await con.expect(/VPCS>\s*$/, { timeout: 40000 });
    return con.output;
  } finally { con.close(); }
}

export const bake = ["r1", "r2", "r3"];

export async function verify(lab, check, { imported = false } = {}) {
  await lab.startAll();
  if (!imported) await configure(lab);
  const r1 = await routerOs(lab, lab.node("r1"));
  let out = "";
  for (let i = 0; i < 20; i += 1) { out = await r1.run("/routing ospf neighbor print", undefined, 20000); if ((out.match(/Full/g) ?? []).length >= 2) break; await sleep(5000); }
  check("R1 has two OSPF neighbours in Full state", (out.match(/Full/g) ?? []).length >= 2);
  let direct = "";
  for (let i = 0; i < 12 && !/ttl=62/.test(direct); i += 1) { direct = await ping(lab, "pc1", "10.3.3.10"); await sleep(3000); }
  check("PC1 reaches PC2 over the direct R1-R3 path (TTL 62)", /ttl=62/.test(direct));
  if (!/ttl=62/.test(direct)) console.log("    ping output:", JSON.stringify(direct.slice(-300)));
  await lab.api("DELETE", `/projects/${lab.projectId}/links/${(await lab.findLink("r1", "r3")).link_id}`);
  let after = "";
  for (let i = 0; i < 20 && !/ttl=61/.test(after); i += 1) { await sleep(5000); after = await ping(lab, "pc1", "10.3.3.10"); }
  check("after cutting R1-R3, OSPF reroutes through R2 (TTL 61)", /ttl=61/.test(after));
  r1.close();
}
