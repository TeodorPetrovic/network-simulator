import { sh, ovsCommand } from "./lib.mjs";

export const id = "02-ovs-ryu";
export const title = "Open vSwitch with a Ryu OpenFlow controller";
export const summary = "An OpenFlow 1.3 switch whose forwarding is programmed by a Ryu learning-switch app over an out-of-band management network.";
export const appliances = ["openvswitch-management.gns3a", "webterm.gns3a"];
export const needs = ["osrg/ryu:latest"];
export const readme = `
Built from the registry appliance \`openvswitch-management\` (eth0 is the management port, kept out of br0), the
VPCS nodes as hosts, and a Ryu container (no registry appliance exists; the template comes from the
pulled image). OVS runs in fail-mode secure, so it forwards nothing until Ryu installs flows. Open the Webterm node's console (a browser desktop on the management network) and browse to http://192.168.0.100:8080 for the Ryu topology viewer, which draws the switch and its links; the REST API is on the same port (/stats/switches). Check
\`ovs-ofctl -O OpenFlow13 dump-flows br0\` after pinging.`;

export async function build(lab) {
  await lab.switch("mgmt", { name: "Mgmt-SW", x: 0, y: -150 });
  await lab.imageDevice("ryu", "osrg/ryu:latest", { name: "Ryu", x: -150, y: -250,
    set: { start_command: sh("cd /root/ryu && exec ryu-manager --observe-links ryu/app/gui_topology/gui_topology.py ryu.app.example_switch_13"), console_type: "http", console_http_port: 8080, console_http_path: "/" }, net: { eth0: { ip: "192.168.0.100/24" } } });
  await lab.device("ovs", "openvswitch-management.gns3a", { name: "OVS", x: 0, y: 0, set: { adapters: 4, start_command: ovsCommand(["ovs-vsctl set bridge br0 protocols=OpenFlow13 fail_mode=secure", "ovs-vsctl set-controller br0 tcp:192.168.0.100:6633"]) }, net: { eth0: { ip: "192.168.0.1/24" } } });
  for (let i = 1; i <= 3; i += 1) await lab.pc(`h${i}`, { name: `H${i}`, x: -200 + (i - 1) * 200, y: 180, ip: `10.0.0.${i}/24` });
  await lab.device("term", "webterm.gns3a", { name: "Webterm", x: 150, y: -250, net: { eth0: { ip: "192.168.0.50/24" } } });
  await lab.link(["mgmt", 0], ["ryu", 0]);
  await lab.link(["mgmt", 2], ["term", 0]);
  await lab.link(["mgmt", 1], ["ovs", 0]);
  for (let i = 1; i <= 3; i += 1) await lab.link(["ovs", i], [`h${i}`, 0]);
}

export async function verify(lab, check) {
  await lab.startAll();
  await lab.expectExec("ovs", "ovs-vsctl --bare --columns=is_connected find controller", /true/, { timeout: 90000 });
  check("OVS connected to Ryu", true);
  const ping = await lab.pcPing("h1", "10.0.0.3", { tries: 10 });
  check("hosts ping through controller-programmed switch", /bytes from 10\.0\.0\.3/.test(ping));
  await lab.expectExec("term", "curl -s -o /dev/null -w %{http_code} -m 5 http://192.168.0.100:8080/", /200/, { timeout: 90000 });
  check("Webterm reaches the Ryu topology web UI on :8080", true);
  check("Ryu installed learned flows", /priority=1,.*actions=output/.test(lab.exec("ovs", "ovs-ofctl -O OpenFlow13 dump-flows br0")));
}
