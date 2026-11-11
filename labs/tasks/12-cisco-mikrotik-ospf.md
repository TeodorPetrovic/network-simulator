# Lab 12: OSPF between a Cisco router and a MikroTik, managed with WinBox

**File:** `12-cisco-mikrotik-ospf.gns3project` · **Time:** 90 min · **Nodes:** R1-HQ (Cisco 3725), CHR-Branch (MikroTik CHR), WinBox, PC-HQ, PC-BR, HQ-SW

## Scenario
A company is connecting a new branch office. Headquarters runs a Cisco router (R1-HQ), the branch has a MikroTik router (CHR-Branch). HQ has the Internet uplink (simulated by the loopback 203.0.113.1 on R1). You must make both routers exchange routes with OSPF so the branch can reach HQ and the Internet, and you manage the branch router remotely with WinBox **by IP address, not by MAC address**.

## Addressing (mandatory)
| Network | Subnet | Addresses |
|---|---|---|
| HQ LAN | 10.12.1.0/24 | R1 .1, WinBox host .10, PC-HQ .11 |
| Transit link | 10.12.12.0/30 | R1 .1, CHR .2 |
| Branch LAN | 10.12.2.0/24 | CHR .1, PC-BR .10 |
| Router IDs | loopbacks | R1 1.1.1.1, CHR 2.2.2.2 |
| "Internet" | 203.0.113.1/32 | R1 Loopback1 |

OSPF: process 1, **area 0** everywhere.

## Tasks
1. Start the lab. Open R1's console and the WinBox node's console (VNC).
2. Check the HQ side: `show ip ospf neighbor`, `show ip route ospf`, `show ip ospf interface fa0/1`.
3. **Connect with WinBox using the IP address.** In WinBox type `10.12.2.1` in *Connect To*, user `admin`, empty password. Do **not** use the Neighbors tab (that is MAC-based). Explain why the IP connection only works once OSPF has converged (the WinBox host is at HQ, the CHR is in another subnet) and why a MAC connection would have worked even without routing.
4. In WinBox open *Routing → OSPF → Neighbors* and *IP → Routes*. Find the OSPF neighbour (R1, state Full), the learned HQ LAN and the default route 0.0.0.0/0. Note which one is E2.
5. Set an admin password in WinBox (*System → Users*) and reconnect by IP.
6. Ping PC-BR → PC-HQ and PC-BR → 203.0.113.1. Record the TTLs.
7. Rebuild the OSPF configuration yourself to prove you understand it: on the CHR remove the OSPF instance (*Routing → OSPF*) and recreate it in WinBox with instance, area, and interface templates for 10.12.12.0/30, 10.12.2.0/24 (passive) and the loopback. Return to Full. If your WinBox session drops, explain why.
8. Change the OSPF cost on R1 Fa0/1 (`ip ospf cost 50`) and on the CHR template. Find the cost shown on the other side.
9. Make the interfaces of both routers authenticate with a shared key (simple or MD5; the CHR template has an *Auth* field). Show that the adjacency drops when the keys differ.
10. Add a second branch network, `10.12.3.0/24`, as a loopback bridge on the CHR, and verify R1 learns it.

## Deliverables
- WinBox screenshots of the neighbour list, route table and the connection dialog showing the IP address.
- `show ip ospf neighbor` and `show ip route ospf` from R1.
- Ping results with TTLs and a note of which are E2/default routes.
- A short answer to the IP-versus-MAC WinBox question from task 3.

## Hints
- Both routers must agree on area, network type (broadcast on Ethernet), hello/dead timers and authentication.
- RouterOS 7 uses `instance`, `area` and `interface-template`; Cisco uses `network … area 0`.
- A DR is elected on the transit link. Compare the roles on both sides.
