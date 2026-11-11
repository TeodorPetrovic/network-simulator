# Lab 1: VLANs on Open vSwitch

**File:** `01-ovs-vlans.gns3project` · **Time:** 45 min · **Nodes:** 2 OVS, 4 VPCS hosts, 1 Linux gateway

## Scenario
Two switches carry two departments: Sales (VLAN 10) and Engineering (VLAN 20). Hosts in the same VLAN must reach each other across the trunk. Hosts in different VLANs must only communicate through the gateway.

| Host | VLAN | Address |
|---|---|---|
| H1 / H3 | 10 | 10.10.10.11 / .12, gateway 10.10.10.1 |
| H2 / H4 | 20 | 10.10.20.11 / .12, gateway 10.10.20.1 |

## Tasks
1. Import the project and start every node.
2. On OVS1 and OVS2 list the ports (`ovs-vsctl show`) and the VLAN settings (`ovs-vsctl list port`). Write down which ports are access ports and which are trunks.
3. From H1 ping H3. From H2 ping H4. Both must succeed.
4. From H1 ping H2 and trace the path (`trace 10.10.20.11` on VPCS). It must go through 10.10.10.1.
5. Move H3's switch port to VLAN 20 (`ovs-vsctl set port ethN tag=20`). Predict, then test, which pings now work.
6. Remove VLAN 20 from the trunk between the switches (`trunks=10`). Which pings break and why?
7. Restore the original configuration.

## Deliverables
- A table of every port: mode, VLAN(s).
- The output of one successful same-VLAN ping and one inter-VLAN ping with TTLs. Explain why the TTLs differ.
- A short answer: what does the gateway need (and why does it use sub-interfaces `eth0.10` and `eth0.20`)?

## Hints
Capture on the trunk link (right-click the link, Start capture) and find the 802.1Q tag in Wireshark.
