import { bootIos, routerOs, telnet, sleep } from "./lib.mjs";

export const id = "12-cisco-mikrotik-ospf";
export const title = "OSPF between a Cisco router and a MikroTik, managed with WinBox over IP";
export const summary = "HQ Cisco R1 and branch MikroTik CHR peer in OSPF area 0; HQ default route is redistributed; WinBox manages the CHR by IP address.";
export const appliances = ["cisco-3725.gns3a", "mikrotik-chr.gns3a", "mikrotik-winbox.gns3a"];
export const readme = `
HQ LAN 10.12.1.0/24 (R1 .1, WinBox host .10, PC-HQ .11). Transit link R1 Fa0/1 - CHR ether1 10.12.12.0/30 (R1 .1, CHR .2).
Branch LAN 10.12.2.0/24 (CHR ether2 .1, PC-BR .10). R1 Lo0 1.1.1.1, CHR lo0 2.2.2.2, R1 Lo1 203.0.113.1 stands for the Internet.
WinBox: connect to 10.12.2.1 (type the IP in "Connect To", do not use the Neighbors/MAC list), login admin, empty password.
Task sheet: labs/tasks/12-cisco-mikrotik-ospf.md.`;

const r1Config = [
  "hostname R1", "!", "no ip domain-lookup", "!",
  "interface Loopback0", " ip address 1.1.1.1 255.255.255.255", "!",
  "interface Loopback1", " description Simulated Internet", " ip address 203.0.113.1 255.255.255.255", "!",
  "interface FastEthernet0/0", " ip address 10.12.1.1 255.255.255.0", " no shutdown", "!",
  "interface FastEthernet0/1", " ip address 10.12.12.1 255.255.255.252", " no shutdown", "!",
  "router ospf 1", " router-id 1.1.1.1", " network 10.12.1.0 0.0.0.255 area 0", " network 10.12.12.0 0.0.0.3 area 0", " network 1.1.1.1 0.0.0.0 area 0",
  " passive-interface FastEthernet0/0", " default-information originate always", "!",
  "ip route 0.0.0.0 0.0.0.0 Loopback1", "!", "end", "",
].join("\n");

export async function build(lab) {
  await lab.switch("hq", { name: "HQ-SW", x: -250, y: -100 });
  await lab.device("r1", "cisco-3725.gns3a", { name: "R1-HQ", x: 0, y: 0, set: { startup_config: r1Config } });
  await lab.device("chr", "mikrotik-chr.gns3a", { name: "CHR-Branch", x: 300, y: 0, image: "mikrotik-chr-7.23.8", set: { adapters: 2 } });
  await lab.device("winbox", "mikrotik-winbox.gns3a", { name: "WinBox", x: -450, y: -100, net: { eth0: { ip: "10.12.1.10/24", gw: "10.12.1.1" } } });
  await lab.pc("pchq", { name: "PC-HQ", x: -250, y: 100, ip: "10.12.1.11/24", gw: "10.12.1.1" });
  await lab.pc("pcbr", { name: "PC-BR", x: 550, y: 0, ip: "10.12.2.10/24", gw: "10.12.2.1" });
  await lab.link(["winbox", 0], ["hq", 0]);
  await lab.link(["pchq", 0], ["hq", 1]);
  await lab.link(["r1", "0/0"], ["hq", 2]);
  await lab.link(["r1", "0/1"], ["chr", 0]);
  await lab.link(["chr", 1], ["pcbr", 0]);
}

export async function configure(lab) {
  const r = await routerOs(lab, lab.node("chr"));
  try {
    for (const line of [
      "/system identity set name=CHR-Branch",
      "/ip address add address=10.12.12.2/30 interface=ether1",
      "/ip address add address=10.12.2.1/24 interface=ether2",
      "/ip service set winbox disabled=no",
      "/interface bridge add name=lo0",
      "/ip address add address=2.2.2.2/32 interface=lo0",
      "/routing ospf instance add name=o1 version=2 router-id=2.2.2.2",
      "/routing ospf area add name=backbone area-id=0.0.0.0 instance=o1",
      "/routing ospf interface-template add area=backbone networks=10.12.12.0/30",
      "/routing ospf interface-template add area=backbone networks=10.12.2.0/24 passive",
      "/routing ospf interface-template add area=backbone networks=2.2.2.2/32 passive",
    ]) await r.run(line);
  } finally { r.close(); }
}

export const bake = ["chr"];

async function ping(lab, key, target) {
  const con = await telnet(lab.node(key));
  try {
    con.send("\r"); await con.expect(/VPCS>/); con.clear();
    con.send(`ping ${target} -c 3\r`);
    await con.expect(/VPCS>\s*$/, { timeout: 40000 });
    return con.output;
  } finally { con.close(); }
}

export async function verify(lab, check, { imported = false } = {}) {
  await lab.startAll();
  if (!imported) await configure(lab);
  const r1 = await bootIos(lab.node("r1"));
  let nbr = "";
  for (let i = 0; i < 30; i += 1) { nbr = await r1.run("show ip ospf neighbor"); if (/FULL/.test(nbr)) break; await sleep(5000); }
  r1.close();
  check("R1 has the CHR as a FULL OSPF neighbour", /FULL/.test(nbr));
  const chr = await routerOs(lab, lab.node("chr"));
  let routes = "";
  for (let i = 0; i < 12 && !/0\.0\.0\.0\/0/.test(routes); i += 1) { routes = await chr.run("/ip route print where ospf"); if (!/0\.0\.0\.0\/0/.test(routes)) await sleep(5000); }
  chr.close();
  check("CHR learns the default route via OSPF", /0\.0\.0\.0\/0/.test(routes));
  check("CHR learns the HQ LAN via OSPF", /10\.12\.1\.0\/24/.test(routes));
  await lab.expectExec("winbox", "timeout 3 bash -c 'echo > /dev/tcp/10.12.2.1/8291' && echo open", /open/, { timeout: 60000 });
  check("WinBox host reaches the CHR by IP (10.12.2.1 tcp/8291) across OSPF", true);
  let out = "";
  for (let i = 0; i < 8 && !/ttl=62/.test(out); i += 1) { out = await ping(lab, "pcbr", "10.12.1.11"); await sleep(3000); }
  check("PC-BR reaches PC-HQ across Cisco and MikroTik (TTL 62)", /ttl=62/.test(out));
}
