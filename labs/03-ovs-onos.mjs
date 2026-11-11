import { ovsCommand } from "./lib.mjs";

export const id = "03-ovs-onos";
export const title = "ONOS SDN controller managing two Open vSwitches";
export const summary = "ONOS (OpenFlow 1.3, reactive forwarding) discovers two OVS switches, their link and two hosts.";
export const appliances = ["onos.gns3a", "openvswitch-management.gns3a", "webterm.gns3a"];
export const readme = `
Built from the registry appliances \`onos\`, \`openvswitch-management\` and \`alpine-docker\`. ONOS needs about a minute
to boot. Its GUI/REST are on 192.168.0.200:8181 (user \`onos\`, password \`rocks\`) on the management switch; use
\`curl -u onos:rocks localhost:8181/onos/v1/devices\` from the ONOS console. Ping h1 -> h2 to trigger host discovery.`;

export async function build(lab) {
  await lab.switch("mgmt", { name: "Mgmt-SW", x: 0, y: -150 });
  await lab.device("onos", "onos.gns3a", { name: "ONOS", x: -150, y: -250,
    set: { environment: "ONOS_APPS=drivers,openflow,fwd,gui2\nJAVA_OPTS=-Xms256m -Xmx1200m" }, net: { eth0: { ip: "192.168.0.200/24" } } });
  await lab.configure("onos", { console_type: "http", console_http_port: 8181, console_http_path: "/onos/ui" });
  for (const [key, name, x, ip, dpid] of [["ovs1", "OVS1", -150, "192.168.0.1/24", "0000000000000001"], ["ovs2", "OVS2", 150, "192.168.0.2/24", "0000000000000002"]]) {
    await lab.device(key, "openvswitch-management.gns3a", { name, x, y: 0, net: { eth0: { ip } },
      set: { adapters: 3, start_command: ovsCommand([`ovs-vsctl set bridge br0 protocols=OpenFlow13 fail_mode=secure other-config:datapath-id=${dpid}`, "ovs-vsctl set-controller br0 tcp:192.168.0.200:6653"]) } });
  }
  await lab.pc("h1", { name: "H1", x: -250, y: 180, ip: "10.0.0.1/24" });
  await lab.pc("h2", { name: "H2", x: 250, y: 180, ip: "10.0.0.2/24" });
  await lab.device("term", "webterm.gns3a", { name: "Webterm", x: 150, y: -250, net: { eth0: { ip: "192.168.0.50/24" } } });
  await lab.link(["mgmt", 0], ["onos", 0]);
  await lab.link(["mgmt", 3], ["term", 0]);
  await lab.link(["mgmt", 1], ["ovs1", 0]);
  await lab.link(["mgmt", 2], ["ovs2", 0]);
  await lab.link(["ovs1", 1], ["ovs2", 1]);
  await lab.link(["ovs1", 2], ["h1", 0]);
  await lab.link(["ovs2", 2], ["h2", 0]);
}

export async function verify(lab, check) {
  await lab.startAll();
  const rest = (path) => `curl -s -u onos:rocks http://localhost:8181/onos/v1/${path}`;
  await lab.expectExec("onos", rest("devices"), /"available":true.*"available":true/s, { timeout: 300000, interval: 5000 });
  check("ONOS sees both switches", true);
  const ping = await lab.pcPing("h1", "10.0.0.2", { tries: 10 });
  check("h1 reaches h2 through ONOS fwd", /bytes from 10\.0\.0\.2/.test(ping));
  check("ONOS discovered both hosts", (lab.exec("onos", rest("hosts")).match(/"mac"/g) ?? []).length >= 2);
  await lab.expectExec("term", "curl -s -o /dev/null -w %{http_code} -m 5 http://192.168.0.200:8181/onos/ui/", /^(200|30[0-9])$/, { timeout: 60000 });
  check("Webterm reaches the ONOS GUI on :8181/onos/ui", true);
  check("ONOS discovered the inter-switch link", /"src"/.test(lab.exec("onos", rest("links"))));
}
