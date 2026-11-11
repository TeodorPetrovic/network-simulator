import { bootIos, telnet, sleep } from "./lib.mjs";

export const id = "08-cisco-switching";
export const title = "Cisco switches: VLANs, trunks and router-on-a-stick";
export const summary = "Two Cisco 3725s with NM-16ESW switch modules carry VLANs 10/20 over an 802.1Q trunk; a third router does inter-VLAN routing.";
export const appliances = ["cisco-3725.gns3a"];
export const readme = `
SW1/SW2 are c3725 routers with an NM-16ESW EtherSwitch module in slot 1 (ports Fa1/0-15). Fa1/0 is the SW1-SW2 trunk,
Fa1/3 on SW1 trunks to R1 (sub-interfaces Fa0/0.10 and Fa0/0.20). PC1/PC3 are in VLAN 10, PC2/PC4 in VLAN 20.
Try \`show vlan-switch brief\` and \`show interfaces trunk\` on the switches. Configs are pushed with
\`bun scripts/labs.mjs configure 08\` (or deploy --start).`;

const switchConfig = (name, access, trunks) => [
  `hostname ${name}`, "!", "no ip domain-lookup", "!",
  "vlan 10", " name SALES", "vlan 20", " name ENG", "!",
  ...trunks.flatMap((p) => [`interface FastEthernet1/${p}`, " switchport mode trunk", " no shutdown", "!"]),
  ...Object.entries(access).flatMap(([p, v]) => [`interface FastEthernet1/${p}`, " switchport mode access", ` switchport access vlan ${v}`, " no shutdown", "!"]),
  "end", "",
].join("\n");

const routerConfig = [
  "hostname R1", "!", "no ip domain-lookup", "!",
  "interface FastEthernet0/0.10", " encapsulation dot1Q 10", " ip address 10.8.10.1 255.255.255.0", "!",
  "interface FastEthernet0/0.20", " encapsulation dot1Q 20", " ip address 10.8.20.1 255.255.255.0", "!",
  "interface FastEthernet0/0", " no ip address", " no shutdown", "!", "end", "",
].join("\n");

export async function build(lab) {
  await lab.device("r1", "cisco-3725.gns3a", { name: "R1", x: -150, y: -200, set: { startup_config: routerConfig } });
  // As in GNS3: copy the appliance template, add the NM-16ESW switch module and a flash disk
  // (for vlan.dat), then place the new template.
  const base = await lab.applianceTemplate("cisco-3725.gns3a");
  const { template_id: _id, appliance_id: _a, appliance_file: _f, appliance_version: _v, appliance_description: _d, auto_image_template: _auto, ...fields } = base;
  const switchTemplate = await lab.template({ ...fields, name: "Cisco 3725 EtherSwitch (labs)", slot1: "NM-16ESW", disk0: 16 });
  await lab.fromTemplate("sw1", switchTemplate, { name: "SW1", x: -150, y: 0 });
  await lab.fromTemplate("sw2", switchTemplate, { name: "SW2", x: 150, y: 0 });
  await lab.configure("sw1", { startup_config: switchConfig("SW1", { 1: 10, 2: 20 }, [0, 3]) });
  await lab.configure("sw2", { startup_config: switchConfig("SW2", { 1: 10, 2: 20 }, [0]) });
  for (const [key, x, ip] of [["pc1", -300, "ip 10.8.10.11/24 10.8.10.1"], ["pc2", -150, "ip 10.8.20.11/24 10.8.20.1"], ["pc3", 150, "ip 10.8.10.12/24 10.8.10.1"], ["pc4", 300, "ip 10.8.20.12/24 10.8.20.1"]]) {
    await lab.vpcs(key, { name: key.toUpperCase(), x, y: 150 });
    await lab.configure(key, { startup_script: ip });
  }
  await lab.link(["sw1", "1/0"], ["sw2", "1/0"]);
  await lab.link(["sw1", "1/3"], ["r1", "0/0"]);
  await lab.link(["sw1", "1/1"], ["pc1", 0]);
  await lab.link(["sw1", "1/2"], ["pc2", 0]);
  await lab.link(["sw2", "1/1"], ["pc3", 0]);
  await lab.link(["sw2", "1/2"], ["pc4", 0]);
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
  const vlans = await sw1.run("show vlan-switch brief");
  sw1.close();
  check("SW1 has VLAN 10 and 20", /SALES/.test(vlans) && /ENG/.test(vlans));
  check("PC1 reaches PC3 in the same VLAN over the trunk (TTL 64)", await pingFrom(lab, "pc1", "10.8.10.12", /ttl=64/));
  check("PC1 reaches PC4 in VLAN 20 via router-on-a-stick (TTL 63)", await pingFrom(lab, "pc1", "10.8.20.12", /ttl=63/));
}
