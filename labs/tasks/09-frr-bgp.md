# Lab 9: eBGP between three autonomous systems (FRR)

**File:** `09-frr-bgp.gns3project` · **Time:** 60 min · **Nodes:** R1, R2, R3 (FRR containers), HostA, HostB

## Scenario
R1 (AS 65001), R2 (AS 65002) and R3 (AS 65003) are connected in a line with eBGP sessions on 10.9.12.0/30 and 10.9.23.0/30. HostA (192.168.1.10) is behind R1 and HostB (192.168.3.10) behind R3.

## Tasks
1. Start the nodes and open a console on each router. Enter `vtysh`.
2. Show the BGP sessions (`show ip bgp summary`) and the BGP table (`show ip bgp`). Explain the AS path of each prefix.
3. Ping HostA → HostB (TTL 61).
4. On R2 add a prefix list and route map that blocks `192.168.3.0/24` towards R1. Show the table on R1 and the failed ping.
5. Remove the filter. Then add AS-path prepending on R3's announcements and see how the path changes on R1.
6. Stop R2's BGP session (`neighbor ... shutdown`) and explain the result.
7. Save the configuration (`write memory`).

## Deliverables
- BGP tables before and after each change.
- The route map and prefix list you wrote.
- An answer to: why does FRR need `no bgp ebgp-requires-policy` in this lab, and what does it do in production?
