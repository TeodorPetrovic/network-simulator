# Vezba 5, Zadatak 1 - test log (for the maintainer only)

Tested on real containers: `gns3/openvswitch` (OVS 2.17) as OVS1 and `alpine` (busybox 1.37) as PC1, PC2, PC3, SERVER.
All names are prefixed `v5t1_`. The whole `resenje.frag.html` was executed twice from a clean state, in order, by a
script that extracts every `<pre data-dev="...">` block from the fragment and runs each line with `docker exec`
(`bash -c` on OVS1, `sh -c` on the alpine nodes; a trailing `&` is run with `docker exec -d`). Both runs gave identical
results (only the order of bridges in `ovs-vsctl show` differs).

## Harness (not part of the exercise)

Links: `v5t1_n1` = OVS1 eth0 - PC1 eth0, `v5t1_n2` = OVS1 eth1 - PC2 eth0, `v5t1_n3` = OVS1 eth2 - PC3 eth0,
`v5t1_n4` = OVS1 eth3 - SERVER eth0. Docker numbers ethN in connect order, so OVS1 is started on n1 and n2..n4 are
connected one by one afterwards, then `/etc/openvswitch/init.sh` is run by hand (it puts all ethN into br0).
(`--network none` cannot be combined with other networks, so the first network is given at `docker run`.)

```
docker network create v5t1_n1 ... v5t1_n4
docker run -d --name v5t1_ovs1 --privileged --network v5t1_n1 --entrypoint sleep gns3/openvswitch infinity
docker network connect v5t1_n2 v5t1_ovs1   (then n3, n4)
docker run -d --name v5t1_pc1 --cap-add NET_ADMIN --network v5t1_n1 alpine sleep infinity   (pc2/n2, pc3/n3, srv/n4)
docker exec v5t1_ovs1 /etc/openvswitch/init.sh
# per PC/SERVER: apk add ethtool; ethtool -K eth0 tx off        <- see deviation below
# per PC/SERVER and OVS1 ethN: ip addr flush dev ethN            (remove the Docker-assigned addresses)
```

## IMPORTANT finding: TCP needs tx checksum offload off on the hosts

Without `ethtool -K eth0 tx off` on the PC/SERVER containers, ping and ARP work but TCP (the HTTP step) does not:
`wget` times out and `/proc/net/snmp` on the SERVER shows `InCsumErrors` growing. Cause: frames leaving the PC veth
carry a partial checksum; OVS (AF_PACKET) re-sends them to the next veth as plain frames, and the receiver drops them.
`ethtool` is not in plain `alpine` or in `gns3/openvswitch`; I installed it with `apk add ethtool` in the harness.
The fragments do NOT mention this. If the real platform (GNS3/ubridge or this project's own link layer) shows the same
symptom, the PC/SERVER images need `ethtool -K eth0 tx off` in their start command. In the GNS3-style runs the
`wget` step 9 cannot work without it.

Other facts: `br-a`/`br-b` were created without `datapath_type` and work (system datapath, host kernel module present).
`ovs-ofctl dump-flows` prints port names quoted when they contain a dash (`output:"patch-a"`). After `del-port patch-a`
the stale flows in `br-a` are not cleaned up (not shown to students).

## Run 1 (clean start, full solution)

```

##### Korak 2 – IP adrese i HTTP server
[pc1]$ ip link set eth0 up
(rc=0)
[pc1]$ ip addr add 10.15.1.11/24 dev eth0
(rc=0)
[pc2]$ ip link set eth0 up
(rc=0)
[pc2]$ ip addr add 10.15.1.12/24 dev eth0
(rc=0)
[pc3]$ ip link set eth0 up
(rc=0)
[pc3]$ ip addr add 10.15.1.13/24 dev eth0
(rc=0)
[srv]$ ip link set eth0 up
(rc=0)
[srv]$ ip addr add 10.15.1.100/24 dev eth0
(rc=0)
[srv]$ echo '#!/bin/sh' > /srv.sh
(rc=0)
[srv]$ echo 'while read l; do [ "${#l}" -le 1 ] && break; done' >> /srv.sh
(rc=0)
[srv]$ echo 'printf "HTTP/1.0 200 OK\r\nContent-Length: 10\r\n\r\nSERVER OK\n"' >> /srv.sh
(rc=0)
[srv]$ chmod +x /srv.sh
(rc=0)
[srv]$ nc -lk -p 80 -e /srv.sh &
(rc=0)

##### Korak 3 – početno stanje
[ovs1]$ ovs-vsctl show
fe3a3e8f-fb46-4df3-8829-63457f350961
    Bridge br0
        datapath_type: netdev
        Port eth0
            Interface eth0
        Port eth1
            Interface eth1
        Port eth2
            Interface eth2
        Port br0
            Interface br0
                type: internal
        Port eth3
            Interface eth3
    Bridge br3
        datapath_type: netdev
        Port br3
            Interface br3
                type: internal
    Bridge br2
        datapath_type: netdev
        Port br2
            Interface br2
                type: internal
    Bridge br1
        datapath_type: netdev
        Port br1
            Interface br1
                type: internal
(rc=0)
[pc1]$ ping -c 2 -W 2 10.15.1.12
PING 10.15.1.12 (10.15.1.12): 56 data bytes
64 bytes from 10.15.1.12: seq=0 ttl=64 time=1.262 ms
64 bytes from 10.15.1.12: seq=1 ttl=64 time=0.539 ms

--- 10.15.1.12 ping statistics ---
2 packets transmitted, 2 packets received, 0% packet loss
round-trip min/avg/max = 0.539/0.900/1.262 ms
(rc=0)
[pc1]$ ping -c 2 -W 2 10.15.1.13
PING 10.15.1.13 (10.15.1.13): 56 data bytes
64 bytes from 10.15.1.13: seq=0 ttl=64 time=1.173 ms
64 bytes from 10.15.1.13: seq=1 ttl=64 time=0.599 ms

--- 10.15.1.13 ping statistics ---
2 packets transmitted, 2 packets received, 0% packet loss
round-trip min/avg/max = 0.599/0.886/1.173 ms
(rc=0)
[pc1]$ ping -c 2 -W 2 10.15.1.100
PING 10.15.1.100 (10.15.1.100): 56 data bytes
64 bytes from 10.15.1.100: seq=0 ttl=64 time=1.277 ms
64 bytes from 10.15.1.100: seq=1 ttl=64 time=0.647 ms

--- 10.15.1.100 ping statistics ---
2 packets transmitted, 2 packets received, 0% packet loss
round-trip min/avg/max = 0.647/0.962/1.277 ms
(rc=0)

##### Korak 4 – dva bridge-a
[ovs1]$ ovs-vsctl del-port br0 eth0
(rc=0)
[ovs1]$ ovs-vsctl del-port br0 eth1
(rc=0)
[ovs1]$ ovs-vsctl del-port br0 eth2
(rc=0)
[ovs1]$ ovs-vsctl del-port br0 eth3
(rc=0)
[ovs1]$ ovs-vsctl add-br br-a
(rc=0)
[ovs1]$ ovs-vsctl add-br br-b
(rc=0)
[ovs1]$ ovs-vsctl add-port br-a eth0
(rc=0)
[ovs1]$ ovs-vsctl add-port br-a eth1
(rc=0)
[ovs1]$ ovs-vsctl add-port br-b eth2
(rc=0)
[ovs1]$ ovs-vsctl add-port br-b eth3
(rc=0)

##### Korak 5 – provera i izolacija bridge-eva
[ovs1]$ ovs-vsctl list-br
br-a
br-b
br0
br1
br2
br3
(rc=0)
[ovs1]$ ovs-vsctl list-ports br-a
eth0
eth1
(rc=0)
[ovs1]$ ovs-vsctl list-ports br-b
eth2
eth3
(rc=0)
[ovs1]$ ovs-vsctl port-to-br eth2
br-b
(rc=0)
[ovs1]$ ovs-vsctl show
fe3a3e8f-fb46-4df3-8829-63457f350961
    Bridge br0
        datapath_type: netdev
        Port br0
            Interface br0
                type: internal
    Bridge br3
        datapath_type: netdev
        Port br3
            Interface br3
                type: internal
    Bridge br-b
        Port eth2
            Interface eth2
        Port eth3
            Interface eth3
        Port br-b
            Interface br-b
                type: internal
    Bridge br2
        datapath_type: netdev
        Port br2
            Interface br2
                type: internal
    Bridge br-a
        Port eth0
            Interface eth0
        Port br-a
            Interface br-a
                type: internal
        Port eth1
            Interface eth1
    Bridge br1
        datapath_type: netdev
        Port br1
            Interface br1
                type: internal
(rc=0)
[pc1]$ ping -c 2 -W 2 10.15.1.12
PING 10.15.1.12 (10.15.1.12): 56 data bytes
64 bytes from 10.15.1.12: seq=0 ttl=64 time=0.669 ms
64 bytes from 10.15.1.12: seq=1 ttl=64 time=0.102 ms

--- 10.15.1.12 ping statistics ---
2 packets transmitted, 2 packets received, 0% packet loss
round-trip min/avg/max = 0.102/0.385/0.669 ms
(rc=0)
[pc1]$ ping -c 2 -W 2 10.15.1.13
PING 10.15.1.13 (10.15.1.13): 56 data bytes

--- 10.15.1.13 ping statistics ---
2 packets transmitted, 0 packets received, 100% packet loss
(rc=1)
[pc1]$ ping -c 2 -W 2 10.15.1.100
PING 10.15.1.100 (10.15.1.100): 56 data bytes

--- 10.15.1.100 ping statistics ---
2 packets transmitted, 0 packets received, 100% packet loss
(rc=1)

##### Korak 6 – patch veza između bridge-eva
[ovs1]$ ovs-vsctl add-port br-a patch-a -- set interface patch-a type=patch options:peer=patch-b
(rc=0)
[ovs1]$ ovs-vsctl add-port br-b patch-b -- set interface patch-b type=patch options:peer=patch-a
(rc=0)
[ovs1]$ ovs-vsctl list-ports br-a
eth0
eth1
patch-a
(rc=0)
[ovs1]$ ovs-vsctl get interface patch-a type
patch
(rc=0)
[ovs1]$ ovs-vsctl get interface patch-a options
{peer=patch-b}
(rc=0)
[ovs1]$ ovs-vsctl get interface patch-b options
{peer=patch-a}
(rc=0)
[pc1]$ ping -c 2 -W 2 10.15.1.13
PING 10.15.1.13 (10.15.1.13): 56 data bytes
64 bytes from 10.15.1.13: seq=0 ttl=64 time=0.942 ms
64 bytes from 10.15.1.13: seq=1 ttl=64 time=0.094 ms

--- 10.15.1.13 ping statistics ---
2 packets transmitted, 2 packets received, 0% packet loss
round-trip min/avg/max = 0.094/0.518/0.942 ms
(rc=0)
[pc1]$ ping -c 2 -W 2 10.15.1.100
PING 10.15.1.100 (10.15.1.100): 56 data bytes
64 bytes from 10.15.1.100: seq=0 ttl=64 time=0.622 ms
64 bytes from 10.15.1.100: seq=1 ttl=64 time=0.085 ms

--- 10.15.1.100 ping statistics ---
2 packets transmitted, 2 packets received, 0% packet loss
round-trip min/avg/max = 0.085/0.353/0.622 ms
(rc=0)
[pc2]$ ping -c 2 -W 2 10.15.1.13
PING 10.15.1.13 (10.15.1.13): 56 data bytes
64 bytes from 10.15.1.13: seq=0 ttl=64 time=0.785 ms
64 bytes from 10.15.1.13: seq=1 ttl=64 time=0.087 ms

--- 10.15.1.13 ping statistics ---
2 packets transmitted, 2 packets received, 0% packet loss
round-trip min/avg/max = 0.087/0.436/0.785 ms
(rc=0)
[pc3]$ ping -c 2 -W 2 10.15.1.100
PING 10.15.1.100 (10.15.1.100): 56 data bytes
64 bytes from 10.15.1.100: seq=0 ttl=64 time=0.726 ms
64 bytes from 10.15.1.100: seq=1 ttl=64 time=0.129 ms

--- 10.15.1.100 ping statistics ---
2 packets transmitted, 2 packets received, 0% packet loss
round-trip min/avg/max = 0.129/0.427/0.726 ms
(rc=0)

##### Korak 7 – OpenFlow pravila na br-a
[ovs1]$ ovs-ofctl del-flows br-a
(rc=0)
[ovs1]$ ovs-ofctl add-flow br-a 'priority=200,arp,in_port=eth0,actions=output:eth1,output:patch-a'
(rc=0)
[ovs1]$ ovs-ofctl add-flow br-a 'priority=200,arp,in_port=eth1,actions=output:eth0'
(rc=0)
[ovs1]$ ovs-ofctl add-flow br-a 'priority=200,arp,in_port=patch-a,actions=output:eth0'
(rc=0)
[ovs1]$ ovs-ofctl add-flow br-a 'priority=100,ip,in_port=eth0,nw_dst=10.15.1.12,actions=output:eth1'
(rc=0)
[ovs1]$ ovs-ofctl add-flow br-a 'priority=100,ip,in_port=eth1,nw_dst=10.15.1.11,actions=output:eth0'
(rc=0)
[ovs1]$ ovs-ofctl add-flow br-a 'priority=100,ip,in_port=patch-a,actions=output:eth0'
(rc=0)
[ovs1]$ ovs-ofctl add-flow br-a 'priority=50,icmp,in_port=eth0,actions=output:patch-a'
(rc=0)
[ovs1]$ ovs-ofctl add-flow br-a 'priority=0,actions=drop'
(rc=0)
[pc1]$ ping -c 2 -W 2 10.15.1.12
PING 10.15.1.12 (10.15.1.12): 56 data bytes
64 bytes from 10.15.1.12: seq=0 ttl=64 time=0.586 ms
64 bytes from 10.15.1.12: seq=1 ttl=64 time=0.121 ms

--- 10.15.1.12 ping statistics ---
2 packets transmitted, 2 packets received, 0% packet loss
round-trip min/avg/max = 0.121/0.353/0.586 ms
(rc=0)
[pc1]$ ping -c 2 -W 2 10.15.1.13
PING 10.15.1.13 (10.15.1.13): 56 data bytes
64 bytes from 10.15.1.13: seq=0 ttl=64 time=0.686 ms
64 bytes from 10.15.1.13: seq=1 ttl=64 time=0.087 ms

--- 10.15.1.13 ping statistics ---
2 packets transmitted, 2 packets received, 0% packet loss
round-trip min/avg/max = 0.087/0.386/0.686 ms
(rc=0)
[pc1]$ ping -c 2 -W 2 10.15.1.100
PING 10.15.1.100 (10.15.1.100): 56 data bytes
64 bytes from 10.15.1.100: seq=0 ttl=64 time=0.563 ms
64 bytes from 10.15.1.100: seq=1 ttl=64 time=0.088 ms

--- 10.15.1.100 ping statistics ---
2 packets transmitted, 2 packets received, 0% packet loss
round-trip min/avg/max = 0.088/0.325/0.563 ms
(rc=0)
[pc2]$ ping -c 2 -W 2 10.15.1.11
PING 10.15.1.11 (10.15.1.11): 56 data bytes
64 bytes from 10.15.1.11: seq=0 ttl=64 time=0.083 ms
64 bytes from 10.15.1.11: seq=1 ttl=64 time=0.086 ms

--- 10.15.1.11 ping statistics ---
2 packets transmitted, 2 packets received, 0% packet loss
round-trip min/avg/max = 0.083/0.084/0.086 ms
(rc=0)
[pc2]$ ping -c 2 -W 2 10.15.1.13
PING 10.15.1.13 (10.15.1.13): 56 data bytes

--- 10.15.1.13 ping statistics ---
2 packets transmitted, 0 packets received, 100% packet loss
(rc=1)
[pc2]$ ping -c 2 -W 2 10.15.1.100
PING 10.15.1.100 (10.15.1.100): 56 data bytes

--- 10.15.1.100 ping statistics ---
2 packets transmitted, 0 packets received, 100% packet loss
(rc=1)
[pc3]$ ping -c 2 -W 2 10.15.1.11
PING 10.15.1.11 (10.15.1.11): 56 data bytes
64 bytes from 10.15.1.11: seq=0 ttl=64 time=0.084 ms
64 bytes from 10.15.1.11: seq=1 ttl=64 time=0.097 ms

--- 10.15.1.11 ping statistics ---
2 packets transmitted, 2 packets received, 0% packet loss
round-trip min/avg/max = 0.084/0.090/0.097 ms
(rc=0)
[pc3]$ ping -c 2 -W 2 10.15.1.12
PING 10.15.1.12 (10.15.1.12): 56 data bytes

--- 10.15.1.12 ping statistics ---
2 packets transmitted, 0 packets received, 100% packet loss
(rc=1)

##### Korak 8 – brojači pravila
[ovs1]$ ovs-ofctl --names dump-flows br-a
 cookie=0x0, duration=14.809s, table=0, n_packets=0, n_bytes=0, priority=200,arp,in_port=eth0 actions=output:eth1,output:"patch-a"
 cookie=0x0, duration=14.769s, table=0, n_packets=3, n_bytes=126, priority=200,arp,in_port=eth1 actions=output:eth0
 cookie=0x0, duration=14.727s, table=0, n_packets=6, n_bytes=252, priority=200,arp,in_port="patch-a" actions=output:eth0
 cookie=0x0, duration=14.596s, table=0, n_packets=6, n_bytes=588, priority=100,ip,in_port="patch-a" actions=output:eth0
 cookie=0x0, duration=14.685s, table=0, n_packets=4, n_bytes=392, priority=100,ip,in_port=eth0,nw_dst=10.15.1.12 actions=output:eth1
 cookie=0x0, duration=14.644s, table=0, n_packets=4, n_bytes=392, priority=100,ip,in_port=eth1,nw_dst=10.15.1.11 actions=output:eth0
 cookie=0x0, duration=14.544s, table=0, n_packets=6, n_bytes=588, priority=50,icmp,in_port=eth0 actions=output:"patch-a"
 cookie=0x0, duration=14.500s, table=0, n_packets=6, n_bytes=476, priority=0 actions=drop
(rc=0)
[pc1]$ wget -qO- -T 3 http://10.15.1.100
wget: download timed out
(rc=1)
[ovs1]$ ovs-ofctl --names dump-flows br-a
 cookie=0x0, duration=17.910s, table=0, n_packets=1, n_bytes=42, priority=200,arp,in_port=eth0 actions=output:eth1,output:"patch-a"
 cookie=0x0, duration=17.870s, table=0, n_packets=3, n_bytes=126, priority=200,arp,in_port=eth1 actions=output:eth0
 cookie=0x0, duration=17.828s, table=0, n_packets=7, n_bytes=294, priority=200,arp,in_port="patch-a" actions=output:eth0
 cookie=0x0, duration=17.697s, table=0, n_packets=6, n_bytes=588, priority=100,ip,in_port="patch-a" actions=output:eth0
 cookie=0x0, duration=17.786s, table=0, n_packets=4, n_bytes=392, priority=100,ip,in_port=eth0,nw_dst=10.15.1.12 actions=output:eth1
 cookie=0x0, duration=17.745s, table=0, n_packets=4, n_bytes=392, priority=100,ip,in_port=eth1,nw_dst=10.15.1.11 actions=output:eth0
 cookie=0x0, duration=17.645s, table=0, n_packets=6, n_bytes=588, priority=50,icmp,in_port=eth0 actions=output:"patch-a"
 cookie=0x0, duration=17.601s, table=0, n_packets=9, n_bytes=698, priority=0 actions=drop
(rc=0)

##### Korak 9 – HTTP samo sa PC1 kroz patch vezu
[ovs1]$ ovs-ofctl add-flow br-a 'priority=100,tcp,in_port=eth0,nw_dst=10.15.1.100,tp_dst=80,actions=output:patch-a'
(rc=0)
[pc1]$ wget -qO- -T 3 http://10.15.1.100
SERVER OK
(rc=0)
[pc2]$ wget -qO- -T 3 http://10.15.1.100
wget: download timed out
(rc=1)
[ovs1]$ ovs-ofctl --names dump-flows br-a
 cookie=0x0, duration=21.108s, table=0, n_packets=1, n_bytes=42, priority=200,arp,in_port=eth0 actions=output:eth1,output:"patch-a"
 cookie=0x0, duration=21.068s, table=0, n_packets=6, n_bytes=252, priority=200,arp,in_port=eth1 actions=output:eth0
 cookie=0x0, duration=21.026s, table=0, n_packets=7, n_bytes=294, priority=200,arp,in_port="patch-a" actions=output:eth0
 cookie=0x0, duration=20.895s, table=0, n_packets=11, n_bytes=975, priority=100,ip,in_port="patch-a" actions=output:eth0
 cookie=0x0, duration=20.984s, table=0, n_packets=4, n_bytes=392, priority=100,ip,in_port=eth0,nw_dst=10.15.1.12 actions=output:eth1
 cookie=0x0, duration=20.943s, table=0, n_packets=4, n_bytes=392, priority=100,ip,in_port=eth1,nw_dst=10.15.1.11 actions=output:eth0
 cookie=0x0, duration=3.152s, table=0, n_packets=7, n_bytes=557, priority=100,tcp,in_port=eth0,nw_dst=10.15.1.100,tp_dst=80 actions=output:"patch-a"
 cookie=0x0, duration=20.843s, table=0, n_packets=6, n_bytes=588, priority=50,icmp,in_port=eth0 actions=output:"patch-a"
 cookie=0x0, duration=20.799s, table=0, n_packets=9, n_bytes=698, priority=0 actions=drop
(rc=0)

##### Korak 10 – bez patch veze nema komunikacije
[ovs1]$ ovs-vsctl del-port br-a patch-a
(rc=0)
[pc1]$ ping -c 2 -W 2 10.15.1.13
PING 10.15.1.13 (10.15.1.13): 56 data bytes

--- 10.15.1.13 ping statistics ---
2 packets transmitted, 0 packets received, 100% packet loss
(rc=1)
[pc1]$ wget -qO- -T 3 http://10.15.1.100
wget: download timed out
(rc=1)
[pc1]$ ping -c 2 -W 2 10.15.1.12
PING 10.15.1.12 (10.15.1.12): 56 data bytes
64 bytes from 10.15.1.12: seq=0 ttl=64 time=0.522 ms
64 bytes from 10.15.1.12: seq=1 ttl=64 time=0.084 ms

--- 10.15.1.12 ping statistics ---
2 packets transmitted, 2 packets received, 0% packet loss
round-trip min/avg/max = 0.084/0.303/0.522 ms
(rc=0)
[ovs1]$ ovs-vsctl show
fe3a3e8f-fb46-4df3-8829-63457f350961
    Bridge br0
        datapath_type: netdev
        Port br0
            Interface br0
                type: internal
    Bridge br3
        datapath_type: netdev
        Port br3
            Interface br3
                type: internal
    Bridge br-b
        Port eth2
            Interface eth2
        Port eth3
            Interface eth3
        Port br-b
            Interface br-b
                type: internal
        Port patch-b
            Interface patch-b
                type: patch
                options: {peer=patch-a}
                error: "No usable peer 'patch-a' exists in 'system' datapath."
    Bridge br2
        datapath_type: netdev
        Port br2
            Interface br2
                type: internal
    Bridge br-a
        Port eth0
            Interface eth0
        Port br-a
            Interface br-a
                type: internal
        Port eth1
            Interface eth1
    Bridge br1
        datapath_type: netdev
        Port br1
            Interface br1
                type: internal
(rc=0)
```

## Run 2 (clean start again, full solution)

```

##### Korak 2 – IP adrese i HTTP server
[pc1]$ ip link set eth0 up
(rc=0)
[pc1]$ ip addr add 10.15.1.11/24 dev eth0
(rc=0)
[pc2]$ ip link set eth0 up
(rc=0)
[pc2]$ ip addr add 10.15.1.12/24 dev eth0
(rc=0)
[pc3]$ ip link set eth0 up
(rc=0)
[pc3]$ ip addr add 10.15.1.13/24 dev eth0
(rc=0)
[srv]$ ip link set eth0 up
(rc=0)
[srv]$ ip addr add 10.15.1.100/24 dev eth0
(rc=0)
[srv]$ echo '#!/bin/sh' > /srv.sh
(rc=0)
[srv]$ echo 'while read l; do [ "${#l}" -le 1 ] && break; done' >> /srv.sh
(rc=0)
[srv]$ echo 'printf "HTTP/1.0 200 OK\r\nContent-Length: 10\r\n\r\nSERVER OK\n"' >> /srv.sh
(rc=0)
[srv]$ chmod +x /srv.sh
(rc=0)
[srv]$ nc -lk -p 80 -e /srv.sh &
(rc=0)

##### Korak 3 – početno stanje
[ovs1]$ ovs-vsctl show
b1ac8e20-7bbf-435e-a636-d3177d2b98c1
    Bridge br1
        datapath_type: netdev
        Port br1
            Interface br1
                type: internal
    Bridge br3
        datapath_type: netdev
        Port br3
            Interface br3
                type: internal
    Bridge br0
        datapath_type: netdev
        Port eth1
            Interface eth1
        Port eth0
            Interface eth0
        Port eth2
            Interface eth2
        Port eth3
            Interface eth3
        Port br0
            Interface br0
                type: internal
    Bridge br2
        datapath_type: netdev
        Port br2
            Interface br2
                type: internal
(rc=0)
[pc1]$ ping -c 2 -W 2 10.15.1.12
PING 10.15.1.12 (10.15.1.12): 56 data bytes
64 bytes from 10.15.1.12: seq=0 ttl=64 time=0.951 ms
64 bytes from 10.15.1.12: seq=1 ttl=64 time=0.354 ms

--- 10.15.1.12 ping statistics ---
2 packets transmitted, 2 packets received, 0% packet loss
round-trip min/avg/max = 0.354/0.652/0.951 ms
(rc=0)
[pc1]$ ping -c 2 -W 2 10.15.1.13
PING 10.15.1.13 (10.15.1.13): 56 data bytes
64 bytes from 10.15.1.13: seq=0 ttl=64 time=1.146 ms
64 bytes from 10.15.1.13: seq=1 ttl=64 time=0.626 ms

--- 10.15.1.13 ping statistics ---
2 packets transmitted, 2 packets received, 0% packet loss
round-trip min/avg/max = 0.626/0.886/1.146 ms
(rc=0)
[pc1]$ ping -c 2 -W 2 10.15.1.100
PING 10.15.1.100 (10.15.1.100): 56 data bytes
64 bytes from 10.15.1.100: seq=0 ttl=64 time=1.286 ms
64 bytes from 10.15.1.100: seq=1 ttl=64 time=0.770 ms

--- 10.15.1.100 ping statistics ---
2 packets transmitted, 2 packets received, 0% packet loss
round-trip min/avg/max = 0.770/1.028/1.286 ms
(rc=0)

##### Korak 4 – dva bridge-a
[ovs1]$ ovs-vsctl del-port br0 eth0
(rc=0)
[ovs1]$ ovs-vsctl del-port br0 eth1
(rc=0)
[ovs1]$ ovs-vsctl del-port br0 eth2
(rc=0)
[ovs1]$ ovs-vsctl del-port br0 eth3
(rc=0)
[ovs1]$ ovs-vsctl add-br br-a
(rc=0)
[ovs1]$ ovs-vsctl add-br br-b
(rc=0)
[ovs1]$ ovs-vsctl add-port br-a eth0
(rc=0)
[ovs1]$ ovs-vsctl add-port br-a eth1
(rc=0)
[ovs1]$ ovs-vsctl add-port br-b eth2
(rc=0)
[ovs1]$ ovs-vsctl add-port br-b eth3
(rc=0)

##### Korak 5 – provera i izolacija bridge-eva
[ovs1]$ ovs-vsctl list-br
br-a
br-b
br0
br1
br2
br3
(rc=0)
[ovs1]$ ovs-vsctl list-ports br-a
eth0
eth1
(rc=0)
[ovs1]$ ovs-vsctl list-ports br-b
eth2
eth3
(rc=0)
[ovs1]$ ovs-vsctl port-to-br eth2
br-b
(rc=0)
[ovs1]$ ovs-vsctl show
b1ac8e20-7bbf-435e-a636-d3177d2b98c1
    Bridge br-b
        Port eth3
            Interface eth3
        Port eth2
            Interface eth2
        Port br-b
            Interface br-b
                type: internal
    Bridge br1
        datapath_type: netdev
        Port br1
            Interface br1
                type: internal
    Bridge br-a
        Port eth0
            Interface eth0
        Port eth1
            Interface eth1
        Port br-a
            Interface br-a
                type: internal
    Bridge br3
        datapath_type: netdev
        Port br3
            Interface br3
                type: internal
    Bridge br0
        datapath_type: netdev
        Port br0
            Interface br0
                type: internal
    Bridge br2
        datapath_type: netdev
        Port br2
            Interface br2
                type: internal
(rc=0)
[pc1]$ ping -c 2 -W 2 10.15.1.12
PING 10.15.1.12 (10.15.1.12): 56 data bytes
64 bytes from 10.15.1.12: seq=0 ttl=64 time=0.409 ms
64 bytes from 10.15.1.12: seq=1 ttl=64 time=0.086 ms

--- 10.15.1.12 ping statistics ---
2 packets transmitted, 2 packets received, 0% packet loss
round-trip min/avg/max = 0.086/0.247/0.409 ms
(rc=0)
[pc1]$ ping -c 2 -W 2 10.15.1.13
PING 10.15.1.13 (10.15.1.13): 56 data bytes

--- 10.15.1.13 ping statistics ---
2 packets transmitted, 0 packets received, 100% packet loss
(rc=1)
[pc1]$ ping -c 2 -W 2 10.15.1.100
PING 10.15.1.100 (10.15.1.100): 56 data bytes

--- 10.15.1.100 ping statistics ---
2 packets transmitted, 0 packets received, 100% packet loss
(rc=1)

##### Korak 6 – patch veza između bridge-eva
[ovs1]$ ovs-vsctl add-port br-a patch-a -- set interface patch-a type=patch options:peer=patch-b
(rc=0)
[ovs1]$ ovs-vsctl add-port br-b patch-b -- set interface patch-b type=patch options:peer=patch-a
(rc=0)
[ovs1]$ ovs-vsctl list-ports br-a
eth0
eth1
patch-a
(rc=0)
[ovs1]$ ovs-vsctl get interface patch-a type
patch
(rc=0)
[ovs1]$ ovs-vsctl get interface patch-a options
{peer=patch-b}
(rc=0)
[ovs1]$ ovs-vsctl get interface patch-b options
{peer=patch-a}
(rc=0)
[pc1]$ ping -c 2 -W 2 10.15.1.13
PING 10.15.1.13 (10.15.1.13): 56 data bytes
64 bytes from 10.15.1.13: seq=0 ttl=64 time=0.746 ms
64 bytes from 10.15.1.13: seq=1 ttl=64 time=0.088 ms

--- 10.15.1.13 ping statistics ---
2 packets transmitted, 2 packets received, 0% packet loss
round-trip min/avg/max = 0.088/0.417/0.746 ms
(rc=0)
[pc1]$ ping -c 2 -W 2 10.15.1.100
PING 10.15.1.100 (10.15.1.100): 56 data bytes
64 bytes from 10.15.1.100: seq=0 ttl=64 time=0.634 ms
64 bytes from 10.15.1.100: seq=1 ttl=64 time=0.078 ms

--- 10.15.1.100 ping statistics ---
2 packets transmitted, 2 packets received, 0% packet loss
round-trip min/avg/max = 0.078/0.356/0.634 ms
(rc=0)
[pc2]$ ping -c 2 -W 2 10.15.1.13
PING 10.15.1.13 (10.15.1.13): 56 data bytes
64 bytes from 10.15.1.13: seq=0 ttl=64 time=0.684 ms
64 bytes from 10.15.1.13: seq=1 ttl=64 time=0.085 ms

--- 10.15.1.13 ping statistics ---
2 packets transmitted, 2 packets received, 0% packet loss
round-trip min/avg/max = 0.085/0.384/0.684 ms
(rc=0)
[pc3]$ ping -c 2 -W 2 10.15.1.100
PING 10.15.1.100 (10.15.1.100): 56 data bytes
64 bytes from 10.15.1.100: seq=0 ttl=64 time=0.607 ms
64 bytes from 10.15.1.100: seq=1 ttl=64 time=0.087 ms

--- 10.15.1.100 ping statistics ---
2 packets transmitted, 2 packets received, 0% packet loss
round-trip min/avg/max = 0.087/0.347/0.607 ms
(rc=0)

##### Korak 7 – OpenFlow pravila na br-a
[ovs1]$ ovs-ofctl del-flows br-a
(rc=0)
[ovs1]$ ovs-ofctl add-flow br-a 'priority=200,arp,in_port=eth0,actions=output:eth1,output:patch-a'
(rc=0)
[ovs1]$ ovs-ofctl add-flow br-a 'priority=200,arp,in_port=eth1,actions=output:eth0'
(rc=0)
[ovs1]$ ovs-ofctl add-flow br-a 'priority=200,arp,in_port=patch-a,actions=output:eth0'
(rc=0)
[ovs1]$ ovs-ofctl add-flow br-a 'priority=100,ip,in_port=eth0,nw_dst=10.15.1.12,actions=output:eth1'
(rc=0)
[ovs1]$ ovs-ofctl add-flow br-a 'priority=100,ip,in_port=eth1,nw_dst=10.15.1.11,actions=output:eth0'
(rc=0)
[ovs1]$ ovs-ofctl add-flow br-a 'priority=100,ip,in_port=patch-a,actions=output:eth0'
(rc=0)
[ovs1]$ ovs-ofctl add-flow br-a 'priority=50,icmp,in_port=eth0,actions=output:patch-a'
(rc=0)
[ovs1]$ ovs-ofctl add-flow br-a 'priority=0,actions=drop'
(rc=0)
[pc1]$ ping -c 2 -W 2 10.15.1.12
PING 10.15.1.12 (10.15.1.12): 56 data bytes
64 bytes from 10.15.1.12: seq=0 ttl=64 time=0.599 ms
64 bytes from 10.15.1.12: seq=1 ttl=64 time=0.081 ms

--- 10.15.1.12 ping statistics ---
2 packets transmitted, 2 packets received, 0% packet loss
round-trip min/avg/max = 0.081/0.340/0.599 ms
(rc=0)
[pc1]$ ping -c 2 -W 2 10.15.1.13
PING 10.15.1.13 (10.15.1.13): 56 data bytes
64 bytes from 10.15.1.13: seq=0 ttl=64 time=0.437 ms
64 bytes from 10.15.1.13: seq=1 ttl=64 time=0.084 ms

--- 10.15.1.13 ping statistics ---
2 packets transmitted, 2 packets received, 0% packet loss
round-trip min/avg/max = 0.084/0.260/0.437 ms
(rc=0)
[pc1]$ ping -c 2 -W 2 10.15.1.100
PING 10.15.1.100 (10.15.1.100): 56 data bytes
64 bytes from 10.15.1.100: seq=0 ttl=64 time=0.405 ms
64 bytes from 10.15.1.100: seq=1 ttl=64 time=0.086 ms

--- 10.15.1.100 ping statistics ---
2 packets transmitted, 2 packets received, 0% packet loss
round-trip min/avg/max = 0.086/0.245/0.405 ms
(rc=0)
[pc2]$ ping -c 2 -W 2 10.15.1.11
PING 10.15.1.11 (10.15.1.11): 56 data bytes
64 bytes from 10.15.1.11: seq=0 ttl=64 time=0.096 ms
64 bytes from 10.15.1.11: seq=1 ttl=64 time=0.083 ms

--- 10.15.1.11 ping statistics ---
2 packets transmitted, 2 packets received, 0% packet loss
round-trip min/avg/max = 0.083/0.089/0.096 ms
(rc=0)
[pc2]$ ping -c 2 -W 2 10.15.1.13
PING 10.15.1.13 (10.15.1.13): 56 data bytes

--- 10.15.1.13 ping statistics ---
2 packets transmitted, 0 packets received, 100% packet loss
(rc=1)
[pc2]$ ping -c 2 -W 2 10.15.1.100
PING 10.15.1.100 (10.15.1.100): 56 data bytes

--- 10.15.1.100 ping statistics ---
2 packets transmitted, 0 packets received, 100% packet loss
(rc=1)
[pc3]$ ping -c 2 -W 2 10.15.1.11
PING 10.15.1.11 (10.15.1.11): 56 data bytes
64 bytes from 10.15.1.11: seq=0 ttl=64 time=0.112 ms
64 bytes from 10.15.1.11: seq=1 ttl=64 time=0.100 ms

--- 10.15.1.11 ping statistics ---
2 packets transmitted, 2 packets received, 0% packet loss
round-trip min/avg/max = 0.100/0.106/0.112 ms
(rc=0)
[pc3]$ ping -c 2 -W 2 10.15.1.12
PING 10.15.1.12 (10.15.1.12): 56 data bytes

--- 10.15.1.12 ping statistics ---
2 packets transmitted, 0 packets received, 100% packet loss
(rc=1)

##### Korak 8 – brojači pravila
[ovs1]$ ovs-ofctl --names dump-flows br-a
 cookie=0x0, duration=14.808s, table=0, n_packets=0, n_bytes=0, priority=200,arp,in_port=eth0 actions=output:eth1,output:"patch-a"
 cookie=0x0, duration=14.760s, table=0, n_packets=3, n_bytes=126, priority=200,arp,in_port=eth1 actions=output:eth0
 cookie=0x0, duration=14.712s, table=0, n_packets=6, n_bytes=252, priority=200,arp,in_port="patch-a" actions=output:eth0
 cookie=0x0, duration=14.573s, table=0, n_packets=6, n_bytes=588, priority=100,ip,in_port="patch-a" actions=output:eth0
 cookie=0x0, duration=14.663s, table=0, n_packets=4, n_bytes=392, priority=100,ip,in_port=eth0,nw_dst=10.15.1.12 actions=output:eth1
 cookie=0x0, duration=14.622s, table=0, n_packets=4, n_bytes=392, priority=100,ip,in_port=eth1,nw_dst=10.15.1.11 actions=output:eth0
 cookie=0x0, duration=14.532s, table=0, n_packets=6, n_bytes=588, priority=50,icmp,in_port=eth0 actions=output:"patch-a"
 cookie=0x0, duration=14.486s, table=0, n_packets=6, n_bytes=476, priority=0 actions=drop
(rc=0)
[pc1]$ wget -qO- -T 3 http://10.15.1.100
wget: download timed out
(rc=1)
[ovs1]$ ovs-ofctl --names dump-flows br-a
 cookie=0x0, duration=17.902s, table=0, n_packets=0, n_bytes=0, priority=200,arp,in_port=eth0 actions=output:eth1,output:"patch-a"
 cookie=0x0, duration=17.854s, table=0, n_packets=3, n_bytes=126, priority=200,arp,in_port=eth1 actions=output:eth0
 cookie=0x0, duration=17.806s, table=0, n_packets=6, n_bytes=252, priority=200,arp,in_port="patch-a" actions=output:eth0
 cookie=0x0, duration=17.667s, table=0, n_packets=6, n_bytes=588, priority=100,ip,in_port="patch-a" actions=output:eth0
 cookie=0x0, duration=17.757s, table=0, n_packets=4, n_bytes=392, priority=100,ip,in_port=eth0,nw_dst=10.15.1.12 actions=output:eth1
 cookie=0x0, duration=17.716s, table=0, n_packets=4, n_bytes=392, priority=100,ip,in_port=eth1,nw_dst=10.15.1.11 actions=output:eth0
 cookie=0x0, duration=17.626s, table=0, n_packets=6, n_bytes=588, priority=50,icmp,in_port=eth0 actions=output:"patch-a"
 cookie=0x0, duration=17.580s, table=0, n_packets=9, n_bytes=698, priority=0 actions=drop
(rc=0)

##### Korak 9 – HTTP samo sa PC1 kroz patch vezu
[ovs1]$ ovs-ofctl add-flow br-a 'priority=100,tcp,in_port=eth0,nw_dst=10.15.1.100,tp_dst=80,actions=output:patch-a'
(rc=0)
[pc1]$ wget -qO- -T 3 http://10.15.1.100
SERVER OK
(rc=0)
[pc2]$ wget -qO- -T 3 http://10.15.1.100
wget: download timed out
(rc=1)
[ovs1]$ ovs-ofctl --names dump-flows br-a
 cookie=0x0, duration=21.109s, table=0, n_packets=0, n_bytes=0, priority=200,arp,in_port=eth0 actions=output:eth1,output:"patch-a"
 cookie=0x0, duration=21.061s, table=0, n_packets=6, n_bytes=252, priority=200,arp,in_port=eth1 actions=output:eth0
 cookie=0x0, duration=21.013s, table=0, n_packets=6, n_bytes=252, priority=200,arp,in_port="patch-a" actions=output:eth0
 cookie=0x0, duration=20.874s, table=0, n_packets=11, n_bytes=975, priority=100,ip,in_port="patch-a" actions=output:eth0
 cookie=0x0, duration=20.964s, table=0, n_packets=4, n_bytes=392, priority=100,ip,in_port=eth0,nw_dst=10.15.1.12 actions=output:eth1
 cookie=0x0, duration=20.923s, table=0, n_packets=4, n_bytes=392, priority=100,ip,in_port=eth1,nw_dst=10.15.1.11 actions=output:eth0
 cookie=0x0, duration=3.157s, table=0, n_packets=7, n_bytes=557, priority=100,tcp,in_port=eth0,nw_dst=10.15.1.100,tp_dst=80 actions=output:"patch-a"
 cookie=0x0, duration=20.833s, table=0, n_packets=6, n_bytes=588, priority=50,icmp,in_port=eth0 actions=output:"patch-a"
 cookie=0x0, duration=20.787s, table=0, n_packets=9, n_bytes=698, priority=0 actions=drop
(rc=0)

##### Korak 10 – bez patch veze nema komunikacije
[ovs1]$ ovs-vsctl del-port br-a patch-a
(rc=0)
[pc1]$ ping -c 2 -W 2 10.15.1.13
PING 10.15.1.13 (10.15.1.13): 56 data bytes

--- 10.15.1.13 ping statistics ---
2 packets transmitted, 0 packets received, 100% packet loss
(rc=1)
[pc1]$ wget -qO- -T 3 http://10.15.1.100
wget: download timed out
(rc=1)
[pc1]$ ping -c 2 -W 2 10.15.1.12
PING 10.15.1.12 (10.15.1.12): 56 data bytes
64 bytes from 10.15.1.12: seq=0 ttl=64 time=0.512 ms
64 bytes from 10.15.1.12: seq=1 ttl=64 time=0.081 ms

--- 10.15.1.12 ping statistics ---
2 packets transmitted, 2 packets received, 0% packet loss
round-trip min/avg/max = 0.081/0.296/0.512 ms
(rc=0)
[ovs1]$ ovs-vsctl show
b1ac8e20-7bbf-435e-a636-d3177d2b98c1
    Bridge br-b
        Port eth3
            Interface eth3
        Port eth2
            Interface eth2
        Port patch-b
            Interface patch-b
                type: patch
                options: {peer=patch-a}
                error: "No usable peer 'patch-a' exists in 'system' datapath."
        Port br-b
            Interface br-b
                type: internal
    Bridge br1
        datapath_type: netdev
        Port br1
            Interface br1
                type: internal
    Bridge br-a
        Port eth0
            Interface eth0
        Port eth1
            Interface eth1
        Port br-a
            Interface br-a
                type: internal
    Bridge br3
        datapath_type: netdev
        Port br3
            Interface br3
                type: internal
    Bridge br0
        datapath_type: netdev
        Port br0
            Interface br0
                type: internal
    Bridge br2
        datapath_type: netdev
        Port br2
            Interface br2
                type: internal
(rc=0)
```

## Re-verification (third run, clean state, prefix `v5v1_`)

Same harness and extraction script as above, fresh containers (the earlier `v5t1_*` runs may have been disturbed by a
sibling agent removing containers). Every command of `resenje.frag.html` ran in order. All `rc` values, ping results
(success/100% loss), `ovs-vsctl` outputs (list-br, list-ports, port-to-br, get interface, show incl. the patch-b
`No usable peer 'patch-a' exists in 'system' datapath.` error), `wget` results (timeout, `SERVER OK`, PC2 timeout) matched
the claims. Non-ARP counters matched exactly (ip patch-a 6/588 then 11/975, nw_dst rules 4/392, icmp 6/588, tcp 7/557,
drop 6/476 -> 9/698). Only ARP counters differed (they depend on ARP retry timing, the fragment already says counters vary):
step 8 `arp,in_port=eth1` 5/210 and `arp,in_port="patch-a"` 9/378 (fragment: 4 and 8); step 9 `arp,in_port=eth1` 9/378 (fragment: 7/294).
No change to the fragments was needed.
