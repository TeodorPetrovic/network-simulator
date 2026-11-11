# Lab 3: Two OpenFlow switches controlled by ONOS

**File:** `03-ovs-onos.gns3project` · **Time:** 60 min · **Nodes:** ONOS, 2 OVS, Webterm, management switch, 2 VPCS hosts

## Scenario
ONOS (192.168.0.200) controls two OVS switches over the management network. H1 (10.0.0.1) and H2 (10.0.0.2) hang off different switches, joined by one inter-switch link. ONOS needs about a minute to boot.

## Tasks
1. Start the nodes and wait until ONOS answers: `curl -u onos:rocks http://localhost:8181/onos/v1/devices` on the ONOS console.
2. In the Webterm (Firefox) open `http://192.168.0.200:8181/onos/ui` and log in (`onos` / `rocks`). Find both devices and the link between them in the topology view.
3. Ping H1 → H2. Check that ONOS now shows both hosts (press `H` in the GUI, or `/onos/v1/hosts`).
4. In the GUI select a switch and inspect its flow table. Explain the entries created by the `fwd` application.
5. Disable the forwarding application (`/onos/v1/applications/org.onosproject.fwd/active`, DELETE) and ping again. What happens, and why?
6. Re-enable it. Then install one flow through the REST API that blocks ICMP from H1 to H2 and show it works.
7. Remove the inter-switch link in GNS3 and describe what ONOS reports.

## Deliverables
- Screenshot of the ONOS topology with both hosts.
- The REST commands you used and their outputs.
- A short explanation of reactive forwarding versus a proactively installed flow.
