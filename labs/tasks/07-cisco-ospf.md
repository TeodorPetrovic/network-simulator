# Lab 7: OSPF on Cisco IOS routers

**File:** `07-cisco-ospf.gns3project` · **Time:** 60 min · **Nodes:** R1, R2, R3 (c3725), PC1, PC2
**Requires:** the c3725 IOS image (embedded in the project or in your GNS3 image library).

## Scenario
PC1 (10.1.1.10/24) — R1 — R2 — R3 — PC2 (10.3.3.10/24) in a line. All routers run OSPF process 1, area 0. Loopbacks: 1.1.1.1, 2.2.2.2, 3.3.3.3.

## Tasks
1. Start the routers and wait for the boot to finish (about a minute each).
2. Check with `show ip ospf neighbor`, `show ip route ospf` and `show ip ospf interface brief`. Identify the DR/BDR on each link, if any.
3. Ping PC1 → PC2 and check the TTL (61). Explain it.
4. Change R2's OSPF router-id and observe what happens to the adjacencies (`clear ip ospf process`).
5. Make `fa0/0` on R1 passive and verify R1 still advertises the network.
6. Add a static default route on R3 and redistribute it into OSPF (`default-information originate`). Show it on R1 as `O*E2`.
7. Save the configurations (`write memory`).

## Deliverables
- Neighbour and route outputs from each router.
- The explanation of how the cost of each path is computed.
- The final running configs.
