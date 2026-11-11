import { bootIos, telnet, sleep } from "./lib.mjs";

export const id = "11-router-on-a-stick";
export const title = "Router-on-a-stick with three VLAN subnets and a remote gateway";
export const summary = "One Cisco router routes VLAN 10/20/30 over a single 802.1Q trunk and reaches a remote site through a second router.";
export const appliances = ["cisco-3725.gns3a"];
export const readme = `
Addressing plan (use 10.11.0.0/16 only): VLAN 10 10.11.10.0/24, VLAN 20 10.11.20.0/24, VLAN 30 10.11.30.0/24,
router link 10.11.100.0/30 (R1 .1, ISP .2), remote LAN 172.16.99.0/24 behind ISP.
SW1 Fa1/0 is the trunk to R1 Fa0/0 (sub-interfaces .10/.20/.30). PC1/PC2/PC3 sit in VLAN 10/20/30.
Reference configs are loaded; erase them and rebuild them as the student task describes. Task sheet: labs/tasks/11-router-on-a-stick.md.`;

const switchConfig = [
  "hostname SW1", "!", "no ip domain-lookup", "!",
  "vlan 10", " name USERS", "vlan 20", " name SERVERS", "vlan 30", " name GUESTS", "!",
  "interface FastEthernet1/0", " switchport mode trunk", " no shutdown", "!",
  ...[[1, 10], [2, 20], [3, 30]].flatMap(([p, v]) => [`interface FastEthernet1/${p}`, " switchport mode access", ` switchport access vlan ${v}`, " no shutdown", "!"]),
  "end", "",
].join("\n");

const r1Config = [
  "hostname R1", "!", "no ip domain-lookup", "!",
  ...[10, 20, 30].flatMap((v) => [`interface FastEthernet0/0.${v}`, ` encapsulation dot1Q ${v}`, ` ip address 10.11.${v}.1 255.255.255.0`, "!"]),
  "interface FastEthernet0/0", " no ip address", " no shutdown", "!",
  "interface FastEthernet0/1", " ip address 10.11.100.1 255.255.255.252", " no shutdown", "!",
  "ip route 0.0.0.0 0.0.0.0 10.11.100.2", "!", "end", "",
].join("\n");

const ispConfig = [
  "hostname ISP", "!", "no ip domain-lookup", "!",
  "interface FastEthernet0/0", " ip address 10.11.100.2 255.255.255.252", " no shutdown", "!",
  "interface FastEthernet0/1", " ip address 172.16.99.1 255.255.255.0", " no shutdown", "!",
  "ip route 10.11.0.0 255.255.0.0 10.11.100.1", "!", "end", "",
].join("\n");

export async function build(lab) {
  await lab.device("r1", "cisco-3725.gns3a", { name: "R1", x: 0, y: -200, set: { startup_config: r1Config } });
  await lab.device("isp", "cisco-3725.gns3a", { name: "ISP", x: 300, y: -200, set: { startup_config: ispConfig } });
  const base = await lab.applianceTemplate("cisco-3725.gns3a");
  const { template_id: _id, appliance_id: _a, appliance_file: _f, appliance_version: _v, appliance_description: _d, auto_image_template: _auto, ...fields } = base;
  const switchTemplate = await lab.template({ ...fields, name: "Cisco 3725 EtherSwitch (labs)", slot1: "NM-16ESW", disk0: 16 });
  await lab.fromTemplate("sw1", switchTemplate, { name: "SW1", x: 0, y: 0 });
  await lab.configure("sw1", { startup_config: switchConfig });
  await lab.pc("pc1", { name: "PC1-VLAN10", x: -250, y: 200, ip: "10.11.10.11/24", gw: "10.11.10.1" });
  await lab.pc("pc2", { name: "PC2-VLAN20", x: 0, y: 200, ip: "10.11.20.11/24", gw: "10.11.20.1" });
  await lab.pc("pc3", { name: "PC3-VLAN30", x: 250, y: 200, ip: "10.11.30.11/24", gw: "10.11.30.1" });
  await lab.pc("remote", { name: "REMOTE", x: 600, y: -200, ip: "172.16.99.10/24", gw: "172.16.99.1" });
  await lab.link(["sw1", "1/0"], ["r1", "0/0"]);
  await lab.link(["r1", "0/1"], ["isp", "0/0"]);
  await lab.link(["isp", "0/1"], ["remote", 0]);
  await lab.link(["sw1", "1/1"], ["pc1", 0]);
  await lab.link(["sw1", "1/2"], ["pc2", 0]);
  await lab.link(["sw1", "1/3"], ["pc3", 0]);
}

async function pingFrom(lab, key, target, re, tries = 6) {
  const con = await telnet(lab.node(key));
  try {
    for (let i = 0; i < tries; i += 1) {
      con.send("\r"); await sleep(300); con.clear(); con.send(`ping ${target} -c 3\r`); await sleep(7000);
      if (re.test(con.output)) return true;
    }
    return false;
  } finally { con.close(); }
}

export async function verify(lab, check) {
  await lab.startAll();
  const sw1 = await bootIos(lab.node("sw1"));
  const trunk = await sw1.run("show interfaces trunk");
  sw1.close();
  check("SW1 trunks VLANs 10,20,30 on Fa1/0", /Fa1\/0/.test(trunk) && /10/.test(trunk));
  check("PC1 reaches PC2 across VLANs via R1 (TTL 63)", await pingFrom(lab, "pc1", "10.11.20.11", /ttl=63/));
  check("PC3 reaches PC1 across VLANs via R1 (TTL 63)", await pingFrom(lab, "pc3", "10.11.10.11", /ttl=63/));
  check("PC1 reaches the remote site through R1 and ISP (TTL 62)", await pingFrom(lab, "pc1", "172.16.99.10", /ttl=62/));
}
