import { sh, httpLoop } from "./lib.mjs";
import { existsSync } from "node:fs";

export const id = "10-linux-services";
export const title = "Linux gateway: DHCP, DNS and NAT services";
export const summary = "An Alpine gateway runs dnsmasq (DHCP + DNS) and masquerades a LAN of DHCP clients to the Internet through a NAT node.";
export const appliances = ["alpine-docker.gns3a"];
export const needs = [];
export const builds = [{ tag: "ns-labs/services:latest", context: "labs/docker/services" }];
export const readme = `
Clients (via a start command that waits for their lease) get their address from the gateway's dnsmasq via DHCP (their /etc/network/interfaces says \`inet dhcp\`). The gateway
image is a small Alpine build with dnsmasq + iptables (no registry appliance exists). \`intranet.lab\` resolves to the
internal web server (10.10.0.80). The gateway's eth1 gets DHCP from the host's libvirt NAT network and masquerades the LAN.`;

const gateway = sh([
  "echo 1 > /proc/sys/net/ipv4/ip_forward",
  // the WAN lease can be missed if the NAT peer comes up after us; retry in the background
  "(for i in $(seq 1 40); do ip route | grep -q default && break; ifup -f eth1 >/dev/null 2>&1; sleep 3; done; iptables -t nat -A POSTROUTING -o eth1 -j MASQUERADE) &",
  "mkdir -p /etc/dnsmasq.d",
  "printf '%s\\n' \"interface=eth0\" \"bind-interfaces\" \"dhcp-range=10.10.0.100,10.10.0.150,12h\" \"dhcp-option=3,10.10.0.1\" \"dhcp-option=6,10.10.0.1\" \"host-record=intranet.lab,10.10.0.80\" \"server=8.8.8.8\" \"server=1.1.1.1\" > /etc/dnsmasq.d/lab.conf",
  "exec dnsmasq -k --conf-file=/etc/dnsmasq.d/lab.conf --log-dhcp",
].join("; "));

// docker bind-mounts /etc/resolv.conf, so udhcpc cannot replace it: point the client at the DHCP-supplied router for DNS.
const client = sh([
  "for i in $(seq 1 60); do ip route | grep -q default && break; ifup -f eth0 >/dev/null 2>&1; sleep 3; done",
  "echo \"nameserver $(ip route | awk '/default/{print $3}')\" > /etc/resolv.conf",
  "exec sleep 2147483647",
].join("; "));

export async function build(lab) {
  await lab.switch("lan", { name: "LAN-SW", x: 0, y: 0 });
  await lab.nat("nat", { x: 0, y: -300 });
  // Port definitions live in the project and can be edited later (Node > Configure), as in GNS3.
  await lab.configure("lan", { ports_mapping: Array.from({ length: 8 }, (_, i) => ({ name: `Ethernet${i}`, port_number: i, type: "access", vlan: 1 })) });
  await lab.imageDevice("gw", "ns-labs/services:latest", { name: "Gateway", x: 0, y: -150, set: { adapters: 2, start_command: gateway },
    net: { eth0: { ip: "10.10.0.1/24" }, eth1: "dhcp" } });
  await lab.device("intranet", "alpine-docker.gns3a", { name: "Intranet", x: 250, y: 150, set: { start_command: httpLoop("<h1>intranet.lab</h1>") },
    net: { eth0: { ip: "10.10.0.80/24", gw: "10.10.0.1" } } });
  await lab.device("c1", "alpine-docker.gns3a", { name: "Client1", x: -250, y: 150, set: { start_command: client }, net: { eth0: "dhcp" } });
  await lab.device("c2", "alpine-docker.gns3a", { name: "Client2", x: -100, y: 150, set: { start_command: client }, net: { eth0: "dhcp" } });
  await lab.link(["gw", 0], ["lan", 0]);
  await lab.link(["intranet", 0], ["lan", 1]);
  await lab.link(["c1", 0], ["lan", 2]);
  await lab.link(["c2", 0], ["lan", 3]);
  await lab.link(["gw", 1], ["nat", 0]);
}

export async function verify(lab, check) {
  await lab.startAll();
  await lab.expectExec("c1", "ip addr show eth0", /inet 10\.10\.0\.1\d\d\//, { timeout: 180000, interval: 3000 });
  await lab.expectExec("c2", "cat /etc/resolv.conf", /nameserver 10\.10\.0\.1/, { timeout: 120000, interval: 3000 });
  const addr = lab.exec("c1", "ip addr show eth0");
  check("client got a DHCP lease from the gateway", /inet 10\.10\.0\.1\d\d\//.test(addr));
  check("DHCP supplied the default gateway", /default via 10\.10\.0\.1/.test(lab.exec("c1", "ip route")));
  check("dnsmasq resolves intranet.lab", /10\.10\.0\.80/.test(await lab.expectExec("c1", "nslookup intranet.lab 10.10.0.1 2>&1 || true", /10\.10\.0\.80/, { timeout: 30000 })));
  await lab.expectExec("c2", "wget -qO- -T 5 http://intranet.lab", /intranet\.lab/, { timeout: 30000 });
  check("client browses the internal web server by name", true);
  if (existsSync("/sys/class/net/virbr0")) {
    try { await lab.expectExec("c1", "wget -qO- -T 8 http://example.com", /Example Domain/, { timeout: 90000 }); check("client reaches the Internet through gateway + NAT", true); }
    catch (error) { check("client reaches the Internet through gateway + NAT", false); console.log(`    ${error.message.split("\n").slice(0, 3).join(" | ")}`); }
  }
}
