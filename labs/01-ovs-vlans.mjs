import { ovsCommand } from "./lib.mjs";

export const id = "01-ovs-vlans";
export const title = "Open vSwitch VLANs and trunking";
export const summary = "Two Open vSwitch appliances joined by an 802.1Q trunk, four VLAN hosts, and a router-on-a-stick gateway.";
export const appliances = ["openvswitch.gns3a", "alpine-docker.gns3a"];
export const readme = `
Built from the GNS3 registry appliances \`openvswitch\` and \`alpine-docker\`. End hosts are VPCS nodes (the gateway is a Linux container because it needs 802.1Q sub-interfaces). Hosts h1/h3 are in VLAN 10 and h2/h4 in
VLAN 20; their addresses come from each node's /etc/network/interfaces (Edit network configuration). The OVS ports are
tagged by the lab's configure step (\`ovs-vsctl set port ethN tag=10\`) -- the same commands you would type in the console.`;


export async function build(lab) {
  const ovs = (key, name, adapters, x, ports) => lab.device(key, "openvswitch.gns3a", { name, x, y: 0, set: { adapters, start_command: ovsCommand(ports.map(([n, setting]) => `ovs-vsctl set port eth${n} ${setting}`)) } });
  await ovs("ovs1", "OVS1", 4, -150, [[0, "trunks=10,20"], [1, "tag=10"], [2, "tag=20"], [3, "trunks=10,20"]]);
  await ovs("ovs2", "OVS2", 3, 150, [[0, "trunks=10,20"], [1, "tag=10"], [2, "tag=20"]]);
  await lab.pc("h1", { name: "H1-VLAN10", x: -300, y: -100, ip: "10.10.10.11/24", gw: "10.10.10.1" });
  await lab.pc("h2", { name: "H2-VLAN20", x: -300, y: 100, ip: "10.10.20.11/24", gw: "10.10.20.1" });
  await lab.pc("h3", { name: "H3-VLAN10", x: 300, y: -100, ip: "10.10.10.12/24", gw: "10.10.10.1" });
  await lab.pc("h4", { name: "H4-VLAN20", x: 300, y: 100, ip: "10.10.20.12/24", gw: "10.10.20.1" });
  const alpine = (key, name, x, y, net) => lab.device(key, "alpine-docker.gns3a", { name, x, y, net });
  await alpine("gw", "Gateway", -150, 150, {
    eth0: { up: [] },
    "eth0.10": { ip: "10.10.10.1/24", pre: ["ip link add link eth0 name eth0.10 type vlan id 10"], up: ["echo 1 > /proc/sys/net/ipv4/ip_forward"] },
    "eth0.20": { ip: "10.10.20.1/24", pre: ["ip link add link eth0 name eth0.20 type vlan id 20"] },
  });
  await lab.link(["ovs1", 0], ["ovs2", 0]);
  await lab.link(["ovs1", 1], ["h1", 0]);
  await lab.link(["ovs1", 2], ["h2", 0]);
  await lab.link(["ovs1", 3], ["gw", 0]);
  await lab.link(["ovs2", 1], ["h3", 0]);
  await lab.link(["ovs2", 2], ["h4", 0]);
}

export async function verify(lab, check) {
  await lab.startAll();
  check("OVS VLAN tags applied", /10/.test(await lab.expectExec("ovs1", "ovs-vsctl get port eth1 tag", /10/, { timeout: 120000 })));
  const same = await lab.pcPing("h1", "10.10.10.12", { tries: 12 });
  check("same-VLAN ping across trunk", /bytes from 10\.10\.10\.12/.test(same));
  const routed = await lab.pcPing("h2", "10.10.10.11");
  check("inter-VLAN routing via gateway (TTL 63)", /ttl=63/i.test(routed));
}
