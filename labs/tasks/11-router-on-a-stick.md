# Lab 11: Router-on-a-stick with three VLANs and a remote gateway

**File:** `11-router-on-a-stick.gns3project` · **Time:** 75 min · **Nodes:** SW1 (Cisco EtherSwitch), R1, ISP (Cisco 3725), PC1, PC2, PC3, REMOTE

## Scenario
A small company has three departments on one switch, each in its own VLAN. A single router (R1) routes between them through one trunk cable (router-on-a-stick) and reaches a remote site through the ISP router.

## Addressing rules (mandatory)
Use **only** the block `10.11.0.0/16` for the company. Carve it up exactly like this:

| Purpose | VLAN | Subnet | R1 gateway | Host |
|---|---|---|---|---|
| Users | 10 | 10.11.10.0/24 | 10.11.10.1 | PC1 .11 |
| Servers | 20 | 10.11.20.0/24 | 10.11.20.1 | PC2 .11 |
| Guests | 30 | 10.11.30.0/24 | 10.11.30.1 | PC3 .11 |
| R1 to ISP link | none | 10.11.100.0/30 | R1 .1 / ISP .2 | |
| Remote site (ISP side) | none | 172.16.99.0/24 | ISP .1 | REMOTE .10 |

Do not use any other ranges. The remote site is not part of your block.

## Tasks
1. Start the lab. The nodes come with a working reference configuration. Read it first: `show vlan-switch brief`, `show interfaces trunk` on SW1 and `show ip interface brief` on R1.
2. **Rebuild it yourself.** On SW1, R1 and ISP run `erase startup-config`/clear the running config (or use `default interface`), then configure from scratch using the table above:
   - SW1: create VLANs 10, 20, 30 with names; Fa1/0 as an 802.1Q trunk; Fa1/1, Fa1/2, Fa1/3 as access ports in VLAN 10, 20, 30.
   - R1: sub-interfaces `Fa0/0.10`, `.20`, `.30` with `encapsulation dot1Q <vlan>` and the gateway addresses; Fa0/1 towards the ISP; a default route to the ISP.
   - ISP: its two interfaces and a static route for `10.11.0.0/16` back to R1.
3. Set the three PCs' addresses with the `ip` command (address, mask, gateway).
4. Ping from PC1 to PC2, PC3 and REMOTE. Note the TTL of each (PC1 to PC2 should be 63, to REMOTE 62). Explain why.
5. On R1 run `show ip route` and `show interfaces fa0/0.10`. Explain what the encapsulation line does.
6. Capture on the SW1–R1 link in Wireshark while pinging PC1 to PC2. Find the 802.1Q tag on both the request and the reply and state which VLAN IDs you see.
7. Break it on purpose, then fix it: (a) change PC2's switch port to VLAN 10, (b) remove VLAN 30 from the trunk with `switchport trunk allowed vlan 10,20`, (c) shut `Fa0/0.20` on R1. Record what each fault looks like and how you found it.
8. Add a fourth VLAN 40 (`10.11.40.0/24`, Management) with a new PC and make it reach REMOTE.

## Deliverables
- Final running configs of SW1, R1 and ISP.
- A table of ping results with TTLs.
- A screenshot of the capture showing the 802.1Q tag.
- A fault table (symptom → cause → fix) for task 7.

## Hints
- The trunk must be configured on **both** ends (SW1 Fa1/0 and the sub-interfaces on R1).
- The physical `Fa0/0` interface needs `no shutdown` but no IP address.
- If a PC cannot reach another VLAN, check its gateway first.
