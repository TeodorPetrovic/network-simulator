# Lab 2: OpenFlow switch controlled by Ryu

**File:** `02-ovs-ryu.gns3project` · **Time:** 60 min · **Nodes:** OVS, Ryu controller, Webterm, management switch, 3 VPCS hosts

## Scenario
The OVS switch runs in `secure` fail mode: it forwards nothing until the Ryu controller installs flows. The controller (192.168.0.100) and the Webterm (192.168.0.50) sit on the management network. Hosts H1–H3 are 10.0.0.1–3/24.

## Tasks
1. Start every node. On the OVS console run `ovs-vsctl show` and confirm `is_connected: true` to `tcp:192.168.0.100:6633`.
2. Before any traffic, run `ovs-ofctl -O OpenFlow13 dump-flows br0`. Record what is there.
3. Ping H1 → H3 from the VPCS console. Dump the flows again and explain each new flow (match, priority, actions).
4. Open the Webterm console (VNC), start Firefox and browse to `http://192.168.0.100:8080`. Take a screenshot of the topology viewer showing the switch and the hosts.
5. Stop the Ryu container. Does the ping still work? Delete the flows (`ovs-ofctl -O OpenFlow13 del-flows br0`) and ping again. Explain the difference.
6. Start Ryu again and show that the switch reconnects and traffic flows.
7. Switch OVS to `fail_mode=standalone`, repeat step 5, and compare.

## Deliverables
- Flow tables before and after the ping, with explanations.
- The topology screenshot.
- A paragraph on the differences between `secure` and `standalone` fail modes.

## Hints
The Ryu REST API is on the same port, for example `curl http://192.168.0.100:8080/stats/switches` from the Webterm.
