# Lab 6: OSPF with three MikroTik routers

**File:** `06-mikrotik-ospf.gns3project` · **Time:** 60 min · **Nodes:** R1, R2, R3 (CHR), PC1, PC2

## Scenario
R1, R2 and R3 form a triangle running OSPF area 0. PC1 (10.1.1.10/24) sits behind R1 and PC2 (10.3.3.10/24) behind R3. Point-to-point links use /30 networks (10.0.12.0, 10.0.13.0, 10.0.23.0). Each router has a loopback bridge `lo0` (1.1.1.1, 2.2.2.2, 3.3.3.3).

## Tasks
1. Start every node and open a console on each router.
2. Show the neighbours (`/routing ospf neighbor print`), the LSDB and the route table (`/ip route print`). Draw the topology and mark each router's OSPF interfaces.
3. Ping PC1 → PC2 with `trace`. The path should be R1 → R3 (TTL 62).
4. Raise the OSPF cost on R1's link to R3 so the preferred path goes through R2. Show the new route and TTL (61).
5. Restore the cost, then delete the R1–R3 link in GNS3. Measure how long until traffic recovers, and explain the dead interval.
6. Re-create the link and verify OSPF converges again.
7. Make the PC-facing interfaces passive and explain why.

## Deliverables
- Neighbour tables and route tables before and after each change.
- The measured failover time.
- The configuration commands you typed.
