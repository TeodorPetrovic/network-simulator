# Lab 8: VLANs, trunks and router-on-a-stick on Cisco

**File:** `08-cisco-switching.gns3project` · **Time:** 75 min · **Nodes:** R1, SW1, SW2 (c3725 with NM-16ESW), PC1–PC4
**Requires:** the c3725 IOS image.

## Scenario
SW1 and SW2 are routers with an EtherSwitch module (ports Fa1/0–15). Fa1/0 is the SW1–SW2 trunk and Fa1/3 on SW1 trunks to R1. VLAN 10 (SALES, 10.8.10.0/24) has PC1/PC3; VLAN 20 (ENG, 10.8.20.0/24) has PC2/PC4.

## Tasks
1. Start the devices. On each switch run `show vlan-switch brief` and `show interfaces trunk`.
2. Ping PC1 → PC3 (same VLAN, TTL 64) and PC1 → PC4 (via R1, TTL 63).
3. Look at R1's sub-interfaces `Fa0/0.10` and `Fa0/0.20`. Explain `encapsulation dot1Q` and why the physical interface has no address.
4. Add VLAN 30 (MGMT, 10.8.30.0/24) on both switches, a sub-interface on R1, and a new host. Test it.
5. Shut the trunk on SW1 Fa1/0 and describe which pings fail.
6. Misconfigure the native VLAN on one side of the trunk and observe the log messages.
7. Restore the configuration.

## Deliverables
- Command outputs for each step.
- A topology diagram with VLAN IDs and subnets.
- A paragraph on why router-on-a-stick limits inter-VLAN throughput.
