import { routerOs, sleep } from "./lib.mjs";

export const id = "05-mikrotik-winbox";
export const title = "MikroTik RouterOS managed with WinBox";
export const summary = "A MikroTik CHR router, the registry WinBox desktop appliance (VNC console), and a NAT uplink.";
export const appliances = ["mikrotik-chr.gns3a", "mikrotik-winbox.gns3a"];
export const readme = `
Built from the registry appliances \`mikrotik-chr\` and \`mikrotik-winbox\`. Open the WinBox node's console (VNC) and connect to 10.5.0.1 as \`admin\` with an empty
password (use the Neighbors tab or type the address). The CHR also has Internet through NAT (ether2 + masquerade).
The packaged project ships the CHR disk with this configuration already applied.`;

export async function build(lab) {
  await lab.switch("lan", { name: "LAN-SW", x: 0, y: 0 });
  await lab.nat("nat", { x: 250, y: -150 });
  await lab.device("chr", "mikrotik-chr.gns3a", { name: "CHR", x: 250, y: 0, image: "mikrotik-chr-7.23.8", set: { adapters: 2 } });
  await lab.device("winbox", "mikrotik-winbox.gns3a", { name: "WinBox", x: -250, y: 0, net: { eth0: { ip: "10.5.0.10/24" } } });
  await lab.vpcs("pc", { name: "PC1", x: 0, y: 150 });
  await lab.link(["winbox", 0], ["lan", 0]);
  await lab.link(["chr", 0], ["lan", 1]);
  await lab.link(["pc", 0], ["lan", 2]);
  await lab.link(["chr", 1], ["nat", 0]);
}

export async function configure(lab) {
  const chr = await routerOs(lab, lab.node("chr"));
  try {
    for (const line of [
      "/ip address add address=10.5.0.1/24 interface=ether1",
      "/ip service set winbox disabled=no",
      "/ip dhcp-client add interface=ether2 disabled=no",
      "/ip firewall nat add chain=srcnat out-interface=ether2 action=masquerade",
      "/ip dns set servers=192.168.122.1 allow-remote-requests=yes",
      "/system identity set name=lab-chr",
    ]) await chr.run(line);
  } finally { chr.close(); }
}

export const bake = ["chr"];

export async function verify(lab, check, { imported = false } = {}) {
  await lab.startAll();
  if (!imported) await configure(lab);
  await lab.expectExec("winbox", "timeout 3 bash -c 'echo > /dev/tcp/10.5.0.1/8291' && echo open", /open/, { timeout: 60000 });
  check("WinBox host reaches RouterOS WinBox service (tcp/8291)", true);
  await sleep(3000);
  check("WinBox client is running", /winbox/i.test(lab.exec("winbox", "ps aux")));
  await lab.expectExec("winbox", "ping -c 2 -W 2 10.5.0.1", /2 packets transmitted, 2 (packets )?received/, { timeout: 30000 });
  check("WinBox host pings the router", true);
}
