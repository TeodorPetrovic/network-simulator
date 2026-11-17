# Vezba 6, Zadatak 1 - test log (for the maintainer only)

Tested on real containers: `gns3/openvswitch` (OVS 2.17) as OVS1 and `alpine` as PC1, PC2, SERVER. Names prefixed `v6t1_`.
Every `<pre data-dev="...">` block of `resenje.frag.html` was extracted by a script and each line run with `docker exec`
(`bash -c` on OVS1, `sh -c` on alpine nodes), in order, from a clean environment. Run twice from clean; the pass/fail pattern,
ttl values and neighbor entries were identical (only durations/counters differ slightly).

## Harness (not part of the exercise)

Links: `v6t1_n1` = OVS1 eth0 - PC1 eth0, `v6t1_n2` = OVS1 eth1 - PC2 eth0, `v6t1_n3` = OVS1 eth2 - SERVER eth0.
OVS1 started with `--entrypoint sleep infinity` on n1, n2/n3 connected one by one afterwards (Docker numbers ethN in connect
order), then `/etc/openvswitch/init.sh` run by hand (puts eth0..eth2 in br0). `ip addr flush dev eth0` on all nodes and
on OVS1 ethN removes the Docker-assigned addresses. No ethtool/tx-off needed (only ICMP/ARP).

## Findings

- ARP responder syntax verified: `load:0x020000000101->NXM_NX_ARP_SHA[]`, `load:0x0a100101->NXM_OF_ARP_SPA[]`, `in_port`.
  dump-flows prints the numbers as `0x20000000101` / `0xa100101` and the port as `IN_PORT`.
- Gateway does not answer ping (no ICMP responder); ARP resolves (`ip neigh` shows router MAC).
- `ping -t 1` through the dec_ttl flow gets no reply (packet is dropped; no ICMP time-exceeded is generated, so no traceroute).
- dl_dst flows (150) must beat nw_dst routing flows (100), otherwise PC1->PC2 would be routed (ttl=63).
- `ip neigh flush` prints the flushed entries (noise).

## Run 2 (full output)

```

##### Korak 2 – MAC adrese, IP adrese i podrazumevani izlaz
[pc1]$ ip link set eth0 down
(rc=0)
[pc1]$ ip link set eth0 address 02:00:00:00:01:11
(rc=0)
[pc1]$ ip link set eth0 up
(rc=0)
[pc1]$ ip addr add 10.16.1.11/24 dev eth0
(rc=0)
[pc1]$ ip route add default via 10.16.1.1
(rc=0)
[pc2]$ ip link set eth0 down
(rc=0)
[pc2]$ ip link set eth0 address 02:00:00:00:01:12
(rc=0)
[pc2]$ ip link set eth0 up
(rc=0)
[pc2]$ ip addr add 10.16.1.12/24 dev eth0
(rc=0)
[pc2]$ ip route add default via 10.16.1.1
(rc=0)
[srv]$ ip link set eth0 down
(rc=0)
[srv]$ ip link set eth0 address 02:00:00:00:02:64
(rc=0)
[srv]$ ip link set eth0 up
(rc=0)
[srv]$ ip addr add 10.16.2.100/24 dev eth0
(rc=0)
[srv]$ ip route add default via 10.16.2.1
(rc=0)

##### Korak 3 – početno stanje
[ovs1]$ ovs-vsctl show
8fdb3a0b-0424-462d-9291-53f9a20a782c
    Bridge br3
        datapath_type: netdev
        Port br3
            Interface br3
                type: internal
    Bridge br1
        datapath_type: netdev
        Port br1
            Interface br1
                type: internal
    Bridge br2
        datapath_type: netdev
        Port br2
            Interface br2
                type: internal
    Bridge br0
        datapath_type: netdev
        Port eth1
            Interface eth1
        Port eth0
            Interface eth0
        Port eth2
            Interface eth2
        Port br0
            Interface br0
                type: internal
(rc=0)
[pc1]$ ping -c 2 -W 2 10.16.1.12
PING 10.16.1.12 (10.16.1.12): 56 data bytes
64 bytes from 10.16.1.12: seq=0 ttl=64 time=0.967 ms
64 bytes from 10.16.1.12: seq=1 ttl=64 time=0.434 ms

--- 10.16.1.12 ping statistics ---
2 packets transmitted, 2 packets received, 0% packet loss
round-trip min/avg/max = 0.434/0.700/0.967 ms
(rc=0)
[pc1]$ ping -c 2 -W 2 10.16.2.100
PING 10.16.2.100 (10.16.2.100): 56 data bytes

--- 10.16.2.100 ping statistics ---
2 packets transmitted, 0 packets received, 100% packet loss
(rc=1)

##### Korak 4 – bridge main-net
[ovs1]$ ovs-vsctl del-port br0 eth0
(rc=0)
[ovs1]$ ovs-vsctl del-port br0 eth1
(rc=0)
[ovs1]$ ovs-vsctl del-port br0 eth2
(rc=0)
[ovs1]$ ovs-vsctl add-br main-net
(rc=0)
[ovs1]$ ovs-vsctl add-port main-net eth0
(rc=0)
[ovs1]$ ovs-vsctl add-port main-net eth1
(rc=0)
[ovs1]$ ovs-vsctl add-port main-net eth2
(rc=0)
[ovs1]$ ovs-vsctl list-ports main-net
eth0
eth1
eth2
(rc=0)

##### Korak 5 – brisanje NORMAL pravila
[ovs1]$ ovs-ofctl dump-flows main-net
NXST_FLOW reply (xid=0x4):
 cookie=0x0, duration=0.215s, table=0, n_packets=0, n_bytes=0, idle_age=0, priority=0 actions=NORMAL
(rc=0)
[ovs1]$ ovs-ofctl del-flows main-net
(rc=0)
[ovs1]$ ovs-ofctl dump-flows main-net
NXST_FLOW reply (xid=0x4):
(rc=0)
[pc1]$ ping -c 2 -W 2 10.16.1.12
PING 10.16.1.12 (10.16.1.12): 56 data bytes

--- 10.16.1.12 ping statistics ---
2 packets transmitted, 0 packets received, 100% packet loss
(rc=1)

##### Korak 6 – pravila za komunikaciju unutar iste podmreže
[ovs1]$ ovs-ofctl add-flow main-net 'priority=140,arp,arp_op=1,in_port=eth0,actions=output:eth1'
(rc=0)
[ovs1]$ ovs-ofctl add-flow main-net 'priority=140,arp,arp_op=1,in_port=eth1,actions=output:eth0'
(rc=0)
[ovs1]$ ovs-ofctl add-flow main-net 'priority=150,dl_dst=02:00:00:00:01:11,actions=output:eth0'
(rc=0)
[ovs1]$ ovs-ofctl add-flow main-net 'priority=150,dl_dst=02:00:00:00:01:12,actions=output:eth1'
(rc=0)
[pc1]$ ip neigh flush dev eth0
10.16.1.12 lladdr 02:00:00:00:01:12 ref 1 used 0/0/0 probes 4 REACHABLE

*** Round 1, deleting 1 entries ***
*** Flush is complete after 1 round(s) ***
(rc=0)
[pc1]$ ping -c 2 -W 2 10.16.1.12
PING 10.16.1.12 (10.16.1.12): 56 data bytes
64 bytes from 10.16.1.12: seq=0 ttl=64 time=0.868 ms
64 bytes from 10.16.1.12: seq=1 ttl=64 time=0.566 ms

--- 10.16.1.12 ping statistics ---
2 packets transmitted, 2 packets received, 0% packet loss
round-trip min/avg/max = 0.566/0.717/0.868 ms
(rc=0)
[pc2]$ ip neigh flush dev eth0
10.16.1.11 lladdr 02:00:00:00:01:11 ref 1 used 0/0/0 probes 4 REACHABLE

*** Round 1, deleting 1 entries ***
*** Flush is complete after 1 round(s) ***
(rc=0)

##### Korak 7 – ARP-responder pravila za gateway adrese
[ovs1]$ ovs-ofctl add-flow main-net 'priority=200,arp,arp_op=1,arp_tpa=10.16.1.1,actions=move:NXM_OF_ETH_SRC[]->NXM_OF_ETH_DST[],mod_dl_src:02:00:00:00:01:01,load:0x2->NXM_OF_ARP_OP[],move:NXM_NX_ARP_SHA[]->NXM_NX_ARP_THA[],move:NXM_OF_ARP_SPA[]->NXM_OF_ARP_TPA[],load:0x020000000101->NXM_NX_ARP_SHA[],load:0x0a100101->NXM_OF_ARP_SPA[],in_port'
(rc=0)
[ovs1]$ ovs-ofctl add-flow main-net 'priority=200,arp,arp_op=1,arp_tpa=10.16.2.1,actions=move:NXM_OF_ETH_SRC[]->NXM_OF_ETH_DST[],mod_dl_src:02:00:00:00:02:01,load:0x2->NXM_OF_ARP_OP[],move:NXM_NX_ARP_SHA[]->NXM_NX_ARP_THA[],move:NXM_OF_ARP_SPA[]->NXM_OF_ARP_TPA[],load:0x020000000201->NXM_NX_ARP_SHA[],load:0x0a100201->NXM_OF_ARP_SPA[],in_port'
(rc=0)
[pc1]$ ping -c 1 -W 1 10.16.1.1
PING 10.16.1.1 (10.16.1.1): 56 data bytes

--- 10.16.1.1 ping statistics ---
1 packets transmitted, 0 packets received, 100% packet loss
(rc=1)
[pc1]$ ip neigh show 10.16.1.1
10.16.1.1 dev eth0 lladdr 02:00:00:00:01:01 ref 1 used 0/0/0 probes 4 REACHABLE
(rc=0)
[srv]$ ping -c 1 -W 1 10.16.2.1
PING 10.16.2.1 (10.16.2.1): 56 data bytes

--- 10.16.2.1 ping statistics ---
1 packets transmitted, 0 packets received, 100% packet loss
(rc=1)
[srv]$ ip neigh show 10.16.2.1
10.16.2.1 dev eth0 lladdr 02:00:00:00:02:01 ref 1 used 0/0/0 probes 4 REACHABLE
(rc=0)

##### Korak 8 – pravila za rutiranje
[ovs1]$ ovs-ofctl add-flow main-net 'priority=100,ip,nw_dst=10.16.2.0/24,actions=mod_dl_src:02:00:00:00:02:01,mod_dl_dst:02:00:00:00:02:64,dec_ttl,output:eth2'
(rc=0)
[ovs1]$ ovs-ofctl add-flow main-net 'priority=100,ip,nw_dst=10.16.1.11,actions=mod_dl_src:02:00:00:00:01:01,mod_dl_dst:02:00:00:00:01:11,dec_ttl,output:eth0'
(rc=0)
[ovs1]$ ovs-ofctl add-flow main-net 'priority=100,ip,nw_dst=10.16.1.12,actions=mod_dl_src:02:00:00:00:01:01,mod_dl_dst:02:00:00:00:01:12,dec_ttl,output:eth1'
(rc=0)
[pc1]$ ping -c 2 -W 2 10.16.2.100
PING 10.16.2.100 (10.16.2.100): 56 data bytes
64 bytes from 10.16.2.100: seq=0 ttl=63 time=0.670 ms
64 bytes from 10.16.2.100: seq=1 ttl=63 time=0.116 ms

--- 10.16.2.100 ping statistics ---
2 packets transmitted, 2 packets received, 0% packet loss
round-trip min/avg/max = 0.116/0.393/0.670 ms
(rc=0)
[pc1]$ ping -c 2 -W 2 -t 1 10.16.2.100
PING 10.16.2.100 (10.16.2.100): 56 data bytes

--- 10.16.2.100 ping statistics ---
2 packets transmitted, 0 packets received, 100% packet loss
(rc=1)
[pc2]$ ping -c 2 -W 2 10.16.2.100
PING 10.16.2.100 (10.16.2.100): 56 data bytes
64 bytes from 10.16.2.100: seq=0 ttl=63 time=0.898 ms
64 bytes from 10.16.2.100: seq=1 ttl=63 time=0.085 ms

--- 10.16.2.100 ping statistics ---
2 packets transmitted, 2 packets received, 0% packet loss
round-trip min/avg/max = 0.085/0.491/0.898 ms
(rc=0)
[srv]$ ping -c 2 -W 2 10.16.1.11
PING 10.16.1.11 (10.16.1.11): 56 data bytes
64 bytes from 10.16.1.11: seq=0 ttl=63 time=0.087 ms
64 bytes from 10.16.1.11: seq=1 ttl=63 time=0.090 ms

--- 10.16.1.11 ping statistics ---
2 packets transmitted, 2 packets received, 0% packet loss
round-trip min/avg/max = 0.087/0.088/0.090 ms
(rc=0)

##### Korak 9 – brojači pravila
[ovs1]$ ovs-ofctl --names dump-flows main-net
 cookie=0x0, duration=8.651s, table=0, n_packets=2, n_bytes=84, priority=200,arp,arp_tpa=10.16.1.1,arp_op=1 actions=move:NXM_OF_ETH_SRC[]->NXM_OF_ETH_DST[],mod_dl_src:02:00:00:00:01:01,load:0x2->NXM_OF_ARP_OP[],move:NXM_NX_ARP_SHA[]->NXM_NX_ARP_THA[],move:NXM_OF_ARP_SPA[]->NXM_OF_ARP_TPA[],load:0x20000000101->NXM_NX_ARP_SHA[],load:0xa100101->NXM_OF_ARP_SPA[],IN_PORT
 cookie=0x0, duration=8.608s, table=0, n_packets=1, n_bytes=42, priority=200,arp,arp_tpa=10.16.2.1,arp_op=1 actions=move:NXM_OF_ETH_SRC[]->NXM_OF_ETH_DST[],mod_dl_src:02:00:00:00:02:01,load:0x2->NXM_OF_ARP_OP[],move:NXM_NX_ARP_SHA[]->NXM_NX_ARP_THA[],move:NXM_OF_ARP_SPA[]->NXM_OF_ARP_TPA[],load:0x20000000201->NXM_NX_ARP_SHA[],load:0xa100201->NXM_OF_ARP_SPA[],IN_PORT
 cookie=0x0, duration=9.891s, table=0, n_packets=3, n_bytes=238, priority=150,dl_dst=02:00:00:00:01:11 actions=output:eth0
 cookie=0x0, duration=9.853s, table=0, n_packets=3, n_bytes=238, priority=150,dl_dst=02:00:00:00:01:12 actions=output:eth1
 cookie=0x0, duration=9.981s, table=0, n_packets=1, n_bytes=42, priority=140,arp,in_port=eth0,arp_op=1 actions=output:eth1
 cookie=0x0, duration=9.937s, table=0, n_packets=1, n_bytes=42, priority=140,arp,in_port=eth1,arp_op=1 actions=output:eth0
 cookie=0x0, duration=6.364s, table=0, n_packets=7, n_bytes=686, priority=100,ip,nw_dst=10.16.2.0/24 actions=mod_dl_src:02:00:00:00:02:01,mod_dl_dst:02:00:00:00:02:64,dec_ttl,output:eth2
 cookie=0x0, duration=6.315s, table=0, n_packets=3, n_bytes=294, priority=100,ip,nw_dst=10.16.1.11 actions=mod_dl_src:02:00:00:00:01:01,mod_dl_dst:02:00:00:00:01:11,dec_ttl,output:eth0
 cookie=0x0, duration=6.271s, table=0, n_packets=2, n_bytes=196, priority=100,ip,nw_dst=10.16.1.12 actions=mod_dl_src:02:00:00:00:01:01,mod_dl_dst:02:00:00:00:01:12,dec_ttl,output:eth1
(rc=0)

##### Korak 10 – ACL pravilo
[ovs1]$ ovs-ofctl add-flow main-net 'priority=300,ip,nw_src=10.16.1.12,nw_dst=10.16.2.0/24,actions=drop'
(rc=0)
[pc1]$ ping -c 2 -W 2 10.16.2.100
PING 10.16.2.100 (10.16.2.100): 56 data bytes
64 bytes from 10.16.2.100: seq=0 ttl=63 time=0.420 ms
64 bytes from 10.16.2.100: seq=1 ttl=63 time=0.127 ms

--- 10.16.2.100 ping statistics ---
2 packets transmitted, 2 packets received, 0% packet loss
round-trip min/avg/max = 0.127/0.273/0.420 ms
(rc=0)
[pc2]$ ping -c 2 -W 2 10.16.2.100
PING 10.16.2.100 (10.16.2.100): 56 data bytes

--- 10.16.2.100 ping statistics ---
2 packets transmitted, 0 packets received, 100% packet loss
(rc=1)
[pc2]$ ping -c 2 -W 2 10.16.1.11
PING 10.16.1.11 (10.16.1.11): 56 data bytes
64 bytes from 10.16.1.11: seq=0 ttl=64 time=0.432 ms
64 bytes from 10.16.1.11: seq=1 ttl=64 time=0.103 ms

--- 10.16.1.11 ping statistics ---
2 packets transmitted, 2 packets received, 0% packet loss
round-trip min/avg/max = 0.103/0.267/0.432 ms
(rc=0)
[ovs1]$ ovs-ofctl --names dump-flows main-net | grep drop
 cookie=0x0, duration=5.212s, table=0, n_packets=2, n_bytes=196, priority=300,ip,nw_src=10.16.1.12,nw_dst=10.16.2.0/24 actions=drop
(rc=0)
```
