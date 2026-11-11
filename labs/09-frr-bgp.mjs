import { sh } from "./lib.mjs";

export const id = "09-frr-bgp";
export const title = "FRRouting eBGP across three autonomous systems";
export const summary = "Three FRR routers (AS 65001-65003) exchange LAN and loopback prefixes over eBGP; hosts at both ends talk through AS 65002.";
export const appliances = ["frr-docker.gns3a"];
export const readme = `
Built from the registry appliances \`frr-docker\`, with VPCS nodes as end hosts. Interface addresses live in each node's
/etc/network/interfaces; BGP is configured by /etc/frr/frr.conf, written by each router's start command. Open a router console and run
\`vtysh\`: \`show ip bgp summary\`, \`show ip route bgp\`. HostA (192.168.1.10) pings HostB (192.168.3.10) across AS 65002.`;

const routers = {
  r1: { name: "R1", asn: 65001, rid: "1.1.1.1", x: -250, ifs: { eth0: "10.9.12.1/30", eth1: "192.168.1.1/24" }, nets: ["1.1.1.1/32", "192.168.1.0/24"], peers: [["10.9.12.2", 65002]] },
  r2: { name: "R2", asn: 65002, rid: "2.2.2.2", x: 0, ifs: { eth0: "10.9.12.2/30", eth1: "10.9.23.1/30" }, nets: ["2.2.2.2/32"], peers: [["10.9.12.1", 65001], ["10.9.23.2", 65003]] },
  r3: { name: "R3", asn: 65003, rid: "3.3.3.3", x: 250, ifs: { eth0: "10.9.23.2/30", eth1: "192.168.3.1/24" }, nets: ["3.3.3.3/32", "192.168.3.0/24"], peers: [["10.9.23.1", 65002]] },
};

function frrCommand(r) {
  const conf = ["frr defaults traditional", `hostname ${r.name.toLowerCase()}`, "log stdout", "!", `router bgp ${r.asn}`, ` bgp router-id ${r.rid}`, " no bgp ebgp-requires-policy",
    ...r.peers.map(([ip, as]) => ` neighbor ${ip} remote-as ${as}`), " address-family ipv4 unicast", ...r.nets.map((n) => `  network ${n}`), " exit-address-family", "!"];
  return sh(`printf '%s\\n' ${conf.map((l) => `"${l}"`).join(" ")} > /etc/frr/frr.conf; chown frr:frr /etc/frr/frr.conf; touch /etc/frr/vtysh.conf; exec /usr/lib/frr/docker-start`);
}

export async function build(lab) {
  for (const [key, r] of Object.entries(routers)) {
    const net = Object.fromEntries(Object.entries(r.ifs).map(([ifc, ip]) => [ifc, { ip }]));
    net.lo = { ip: `${r.rid}/32` };
    await lab.device(key, "frr-docker.gns3a", { name: r.name, x: r.x, y: 0, set: { adapters: 2, start_command: frrCommand(r) }, net });
  }
  await lab.pc("ha", { name: "HostA", x: -400, y: 0, ip: "192.168.1.10/24", gw: "192.168.1.1" });
  await lab.pc("hb", { name: "HostB", x: 400, y: 0, ip: "192.168.3.10/24", gw: "192.168.3.1" });
  await lab.link(["ha", 0], ["r1", 1]);
  await lab.link(["r1", 0], ["r2", 0]);
  await lab.link(["r2", 1], ["r3", 0]);
  await lab.link(["r3", 1], ["hb", 0]);
}

export async function verify(lab, check) {
  await lab.startAll();
  const summaryOut = await lab.expectExec("r2", "vtysh -c 'show ip bgp summary'", /10\.9\.12\.1[\s\S]*10\.9\.23\.2/, { timeout: 120000, interval: 4000 });
  check("R2 has BGP sessions to both neighbours", /10\.9\.12\.1/.test(summaryOut) && /10\.9\.23\.2/.test(summaryOut));
  const routes = await lab.expectExec("r1", "vtysh -c 'show ip route bgp'", /192\.168\.3\.0\/24/, { timeout: 90000, interval: 4000 });
  check("R1 learned the AS 65003 LAN via BGP", /192\.168\.3\.0\/24/.test(routes));
  const ping = await lab.pcPing("ha", "192.168.3.10", { tries: 12 });
  check("HostA reaches HostB across three ASes (TTL 61)", /ttl=61/i.test(ping));
}
