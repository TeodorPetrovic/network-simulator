# Test log - Vezba 5, Zadatak 2 (samo za odrzavaoca)

Okruzenje: Docker, 4 kontejnera (v5t2_ovs1 = gns3/openvswitch, OVS 2.17.12; v5t2_pc1, v5t2_pc2, v5t2_srv = alpine), 3 docker mreze v5t2_n1..n3 (jedna mreza = jedan kabl). Kontejneri su pokrenuti sa `sleep infinity`, mreze povezane jedna po jedna (OVS1: n1=eth0, n2=eth1, n3=eth2), pa je pokrenut `/etc/openvswitch/init.sh` (svi ethN u br0, kao u GNS3). Komande resenja su izvrsene automatski, tacno kako su zapisane u resenje.frag.html (skripta cita `<pre><code>` blokove, uredjaj odredjuje po recenici "Na uredjaju X:"). Pun redosled je izvrsen dva puta od cistog stanja (runA, runB); izlazi su se poklapali (brojevi paketa u dump-flows variraju malo).

## Preliminarna provera meter-a (pre pisanja)
- `meter-features`: max_meter:200000 max_bands:1, band_types: drop, capabilities: kbps pktps burst stats.
- Meter radi u ovom datapath-u: 500 pingova (-i 0.01) sa PC2 uz meter rate=5 -> 30 stiglo, 470 odbaceno. Napomena: `ping -A` nije dobar test (ping ceka odgovor pa se sam uspori), zato `-i 0.01`.
- `del-meter` brise i flow koji koristi meter (flow_count:0, flow nestaje iz dump-flows), `mod-meter` ne.
- Pakete koje broji table 0 priority=0 (70 bajta) su IPv6 (MLD): privremeno pravilo `priority=1,ipv6,actions=drop` ih je prebrojalo.
- Napomena: prvi pokusaj (pre ispravke skripte) imao je gresku u harness-u (pogresan uredjaj za neke blokove); rezultati ispod su iz ispravljenih pokretanja.

## Pokretanje A (cisto stanje)


## Korak 1 – Topologija

## Korak 2 – IP i MAC adrese, HTTP server

### PC1
$ ip link set dev eth0 address 00:00:00:00:00:01
$ ip addr flush dev eth0
$ ip addr add 10.15.0.11/24 dev eth0
$ ip addr show eth0
2: eth0@if332: <BROADCAST,MULTICAST,UP,LOWER_UP,M-DOWN> mtu 1500 qdisc noqueue state UP 
    link/ether 00:00:00:00:00:01 brd ff:ff:ff:ff:ff:ff
    inet 10.15.0.11/24 scope global eth0
       valid_lft forever preferred_lft forever


### PC2
$ ip link set dev eth0 address 00:00:00:00:00:02
$ ip addr flush dev eth0
$ ip addr add 10.15.0.12/24 dev eth0
$ ip addr show eth0
2: eth0@if333: <BROADCAST,MULTICAST,UP,LOWER_UP,M-DOWN> mtu 1500 qdisc noqueue state UP 
    link/ether 00:00:00:00:00:02 brd ff:ff:ff:ff:ff:ff
    inet 10.15.0.12/24 scope global eth0
       valid_lft forever preferred_lft forever


### SRV
$ ip link set dev eth0 address 00:00:00:00:00:03
$ ip addr flush dev eth0
$ ip addr add 10.15.0.80/24 dev eth0
$ cat > /srv.sh <<'X'
$ #!/bin/sh
$ while read l; do [ "$l" = "$(printf '\r')" ] && break; done
$ printf 'HTTP/1.0 200 OK\r\nContent-Type: text/plain\r\n\r\nSERVER OK\n'
$ X
$ chmod +x /srv.sh
$ nc -lk -p 80 -e /srv.sh &
$ ip addr show eth0
2: eth0@if334: <BROADCAST,MULTICAST,UP,LOWER_UP,M-DOWN> mtu 1500 qdisc noqueue state UP 
    link/ether 00:00:00:00:00:03 brd ff:ff:ff:ff:ff:ff
    inet 10.15.0.80/24 scope global eth0
       valid_lft forever preferred_lft forever


## Korak 3 – Bridge, OpenFlow 1.3 i prazna tabela pravila

### OVS1
$ ovs-vsctl del-port br0 eth0
$ ovs-vsctl del-port br0 eth1
$ ovs-vsctl del-port br0 eth2
$ ovs-vsctl add-br main-net
$ ovs-vsctl add-port main-net eth0
$ ovs-vsctl add-port main-net eth1
$ ovs-vsctl add-port main-net eth2
$ ovs-vsctl set bridge main-net protocols=OpenFlow13
$ ovs-ofctl -O OpenFlow13 del-flows main-net
$ ovs-ofctl -O OpenFlow13 dump-flows main-net
OFPST_FLOW reply (OF1.3) (xid=0x2):


### PC1
$ ping -c 3 -W 1 10.15.0.80
PING 10.15.0.80 (10.15.0.80): 56 data bytes

--- 10.15.0.80 ping statistics ---
3 packets transmitted, 0 packets received, 100% packet loss


## Korak 4 – Tabela 1: prosleđivanje po MAC adresi

### OVS1
$ ovs-ofctl -O OpenFlow13 add-flow main-net 'table=1,priority=100,dl_dst=00:00:00:00:00:01,actions=output:eth0'
$ ovs-ofctl -O OpenFlow13 add-flow main-net 'table=1,priority=100,dl_dst=00:00:00:00:00:02,actions=output:eth1'
$ ovs-ofctl -O OpenFlow13 add-flow main-net 'table=1,priority=100,dl_dst=00:00:00:00:00:03,actions=output:eth2'
$ ovs-ofctl -O OpenFlow13 add-flow main-net 'table=1,priority=100,dl_dst=ff:ff:ff:ff:ff:ff,actions=flood'
$ ovs-ofctl -O OpenFlow13 add-flow main-net 'table=1,priority=0,actions=drop'
$ ovs-ofctl -O OpenFlow13 dump-flows main-net
OFPST_FLOW reply (OF1.3) (xid=0x2):
 cookie=0x0, duration=0.136s, table=1, n_packets=0, n_bytes=0, priority=100,dl_dst=00:00:00:00:00:01 actions=output:1
 cookie=0x0, duration=0.102s, table=1, n_packets=0, n_bytes=0, priority=100,dl_dst=00:00:00:00:00:02 actions=output:2
 cookie=0x0, duration=0.068s, table=1, n_packets=0, n_bytes=0, priority=100,dl_dst=00:00:00:00:00:03 actions=output:3
 cookie=0x0, duration=0.035s, table=1, n_packets=0, n_bytes=0, priority=100,dl_dst=ff:ff:ff:ff:ff:ff actions=FLOOD
 cookie=0x0, duration=0.002s, table=1, n_packets=0, n_bytes=0, priority=0 actions=drop


### PC1
$ ping -c 3 -W 1 10.15.0.80
PING 10.15.0.80 (10.15.0.80): 56 data bytes

--- 10.15.0.80 ping statistics ---
3 packets transmitted, 0 packets received, 100% packet loss


## Korak 5 – Tabela 0: ACL

### OVS1
$ ovs-ofctl -O OpenFlow13 add-flow main-net 'table=0,priority=100,arp,actions=goto_table:1'
$ ovs-ofctl -O OpenFlow13 add-flow main-net 'table=0,priority=100,icmp,actions=goto_table:1'
$ ovs-ofctl -O OpenFlow13 add-flow main-net 'table=0,priority=100,tcp,in_port=eth0,nw_dst=10.15.0.80,tp_dst=80,actions=goto_table:1'
$ ovs-ofctl -O OpenFlow13 add-flow main-net 'table=0,priority=100,tcp,in_port=eth2,nw_src=10.15.0.80,tp_src=80,actions=goto_table:1'
$ ovs-ofctl -O OpenFlow13 add-flow main-net 'table=0,priority=50,tcp,tp_dst=80,actions=drop'
$ ovs-ofctl -O OpenFlow13 add-flow main-net 'table=0,priority=0,actions=drop'


### PC1
$ ping -c 3 10.15.0.12
$ ping -c 3 10.15.0.80
$ wget -qO- -T 3 http://10.15.0.80/
PING 10.15.0.12 (10.15.0.12): 56 data bytes
64 bytes from 10.15.0.12: seq=0 ttl=64 time=0.745 ms
64 bytes from 10.15.0.12: seq=1 ttl=64 time=0.081 ms
64 bytes from 10.15.0.12: seq=2 ttl=64 time=0.079 ms

--- 10.15.0.12 ping statistics ---
3 packets transmitted, 3 packets received, 0% packet loss
round-trip min/avg/max = 0.079/0.301/0.745 ms
PING 10.15.0.80 (10.15.0.80): 56 data bytes
64 bytes from 10.15.0.80: seq=0 ttl=64 time=0.569 ms
64 bytes from 10.15.0.80: seq=1 ttl=64 time=0.084 ms
64 bytes from 10.15.0.80: seq=2 ttl=64 time=0.096 ms

--- 10.15.0.80 ping statistics ---
3 packets transmitted, 3 packets received, 0% packet loss
round-trip min/avg/max = 0.084/0.249/0.569 ms
SERVER OK


### PC2
$ ping -c 3 10.15.0.80
$ wget -qO- -T 3 http://10.15.0.80/
PING 10.15.0.80 (10.15.0.80): 56 data bytes
64 bytes from 10.15.0.80: seq=0 ttl=64 time=0.388 ms
64 bytes from 10.15.0.80: seq=1 ttl=64 time=0.118 ms
64 bytes from 10.15.0.80: seq=2 ttl=64 time=0.108 ms

--- 10.15.0.80 ping statistics ---
3 packets transmitted, 3 packets received, 0% packet loss
round-trip min/avg/max = 0.108/0.204/0.388 ms
wget: download timed out


## Korak 6 – Brojači pravila

### OVS1
$ ovs-ofctl -O OpenFlow13 dump-flows main-net
OFPST_FLOW reply (OF1.3) (xid=0x2):
 cookie=0x0, duration=9.460s, table=0, n_packets=8, n_bytes=336, priority=100,arp actions=goto_table:1
 cookie=0x0, duration=9.428s, table=0, n_packets=18, n_bytes=1764, priority=100,icmp actions=goto_table:1
 cookie=0x0, duration=9.395s, table=0, n_packets=6, n_bytes=490, priority=100,tcp,in_port=1,nw_dst=10.15.0.80,tp_dst=80 actions=goto_table:1
 cookie=0x0, duration=9.363s, table=0, n_packets=6, n_bytes=459, priority=100,tcp,in_port=3,nw_src=10.15.0.80,tp_src=80 actions=goto_table:1
 cookie=0x0, duration=9.331s, table=0, n_packets=3, n_bytes=222, priority=50,tcp,tp_dst=80 actions=drop
 cookie=0x0, duration=9.300s, table=0, n_packets=3, n_bytes=210, priority=0 actions=drop
 cookie=0x0, duration=12.822s, table=1, n_packets=15, n_bytes=1173, priority=100,dl_dst=00:00:00:00:00:01 actions=output:1
 cookie=0x0, duration=12.788s, table=1, n_packets=8, n_bytes=672, priority=100,dl_dst=00:00:00:00:00:02 actions=output:2
 cookie=0x0, duration=12.754s, table=1, n_packets=12, n_bytes=1078, priority=100,dl_dst=00:00:00:00:00:03 actions=output:3
 cookie=0x0, duration=12.721s, table=1, n_packets=3, n_bytes=126, priority=100,dl_dst=ff:ff:ff:ff:ff:ff actions=FLOOD
 cookie=0x0, duration=12.688s, table=1, n_packets=0, n_bytes=0, priority=0 actions=drop


## Korak 7 – Meter i pravilo koje ga koristi

### OVS1
$ ovs-ofctl -O OpenFlow13 add-meter main-net 'meter=1,pktps,band=type=drop,rate=5'
$ ovs-ofctl -O OpenFlow13 dump-meters main-net
$ ovs-ofctl -O OpenFlow13 add-flow main-net 'table=0,priority=200,icmp,in_port=eth1,actions=meter:1,goto_table:1'
$ ovs-ofctl -O OpenFlow13 dump-flows main-net table=0
OFPST_METER_CONFIG reply (OF1.3) (xid=0x2):
meter=1 pktps bands=
type=drop rate=5
OFPST_FLOW reply (OF1.3) (xid=0x6):
 cookie=0x0, duration=0.031s, table=0, n_packets=0, n_bytes=0, priority=200,icmp,in_port=2 actions=meter:1,goto_table:1
 cookie=0x0, duration=9.582s, table=0, n_packets=18, n_bytes=1764, priority=100,icmp actions=goto_table:1
 cookie=0x0, duration=9.614s, table=0, n_packets=8, n_bytes=336, priority=100,arp actions=goto_table:1
 cookie=0x0, duration=9.517s, table=0, n_packets=6, n_bytes=459, priority=100,tcp,in_port=3,nw_src=10.15.0.80,tp_src=80 actions=goto_table:1
 cookie=0x0, duration=9.549s, table=0, n_packets=6, n_bytes=490, priority=100,tcp,in_port=1,nw_dst=10.15.0.80,tp_dst=80 actions=goto_table:1
 cookie=0x0, duration=9.485s, table=0, n_packets=3, n_bytes=222, priority=50,tcp,tp_dst=80 actions=drop
 cookie=0x0, duration=9.454s, table=0, n_packets=3, n_bytes=210, priority=0 actions=drop


## Korak 8 – Brzi ping i brojači meter-a

### PC2
$ ping -q -i 0.01 -c 500 -W 1 10.15.0.80
PING 10.15.0.80 (10.15.0.80): 56 data bytes

--- 10.15.0.80 ping statistics ---
500 packets transmitted, 30 packets received, 94% packet loss
round-trip min/avg/max = 0.039/0.059/0.092 ms


### PC1
$ ping -q -i 0.01 -c 500 -W 1 10.15.0.80
PING 10.15.0.80 (10.15.0.80): 56 data bytes

--- 10.15.0.80 ping statistics ---
500 packets transmitted, 500 packets received, 0% packet loss
round-trip min/avg/max = 0.025/0.050/0.409 ms


### OVS1
$ ovs-ofctl -O OpenFlow13 meter-stats main-net
$ ovs-ofctl -O OpenFlow13 dump-flows main-net table=0
OFPST_METER reply (OF1.3) (xid=0x2):
meter:1 flow_count:1 packet_in_count:500 byte_in_count:49000 duration:11.735s bands:
0: packet_count:470 byte_count:46060
OFPST_FLOW reply (OF1.3) (xid=0x6):
 cookie=0x0, duration=11.734s, table=0, n_packets=500, n_bytes=49000, priority=200,icmp,in_port=2 actions=meter:1,goto_table:1
 cookie=0x0, duration=21.285s, table=0, n_packets=1026, n_bytes=100548, priority=100,icmp actions=goto_table:1
 cookie=0x0, duration=21.317s, table=0, n_packets=10, n_bytes=420, priority=100,arp actions=goto_table:1
 cookie=0x0, duration=21.220s, table=0, n_packets=6, n_bytes=459, priority=100,tcp,in_port=3,nw_src=10.15.0.80,tp_src=80 actions=goto_table:1
 cookie=0x0, duration=21.252s, table=0, n_packets=6, n_bytes=490, priority=100,tcp,in_port=1,nw_dst=10.15.0.80,tp_dst=80 actions=goto_table:1
 cookie=0x0, duration=21.188s, table=0, n_packets=3, n_bytes=222, priority=50,tcp,tp_dst=80 actions=drop
 cookie=0x0, duration=21.157s, table=0, n_packets=4, n_bytes=280, priority=0 actions=drop


### PC2
$ ping -c 3 10.15.0.80
PING 10.15.0.80 (10.15.0.80): 56 data bytes
64 bytes from 10.15.0.80: seq=0 ttl=64 time=0.080 ms
64 bytes from 10.15.0.80: seq=1 ttl=64 time=0.083 ms
64 bytes from 10.15.0.80: seq=2 ttl=64 time=0.117 ms

--- 10.15.0.80 ping statistics ---
3 packets transmitted, 3 packets received, 0% packet loss
round-trip min/avg/max = 0.080/0.093/0.117 ms


## Korak 9 – Promena brzine meter-a

### OVS1
$ ovs-ofctl -O OpenFlow13 mod-meter main-net 'meter=1,pktps,band=type=drop,rate=50'
$ ovs-ofctl -O OpenFlow13 dump-meters main-net
OFPST_METER_CONFIG reply (OF1.3) (xid=0x2):
meter=1 pktps bands=
type=drop rate=50


### PC2
$ ping -q -i 0.01 -c 500 -W 1 10.15.0.80
PING 10.15.0.80 (10.15.0.80): 56 data bytes

--- 10.15.0.80 ping statistics ---
500 packets transmitted, 309 packets received, 38% packet loss
round-trip min/avg/max = 0.024/0.048/0.115 ms


## Korak 10 – Zamena MAC adrese servera menja samo tabelu 1

### SRV
$ ip link set dev eth0 address 00:00:00:00:00:33


### PC1
$ ip neigh flush dev eth0
10.15.0.12 lladdr 00:00:00:00:00:02 used 0/0/0 probes 4 STALE
10.15.0.80 lladdr 00:00:00:00:00:03 used 0/0/0 probes 4 STALE

*** Round 1, deleting 2 entries ***
*** Flush is complete after 1 round(s) ***


### PC2
$ ip neigh flush dev eth0
10.15.0.11 lladdr 00:00:00:00:00:01 used 0/0/0 probes 1 STALE
10.15.0.80 lladdr 00:00:00:00:00:03 ref 1 used 0/0/0 probes 1 REACHABLE

*** Round 1, deleting 2 entries ***
*** Flush is complete after 1 round(s) ***


### PC1
$ wget -qO- -T 3 http://10.15.0.80/
wget: download timed out


### OVS1
$ ovs-ofctl -O OpenFlow13 dump-flows main-net table=1
OFPST_FLOW reply (OF1.3) (xid=0x6):
 cookie=0x0, duration=36.704s, table=1, n_packets=516, n_bytes=50215, priority=100,dl_dst=00:00:00:00:00:01 actions=output:1
 cookie=0x0, duration=36.670s, table=1, n_packets=352, n_bytes=34272, priority=100,dl_dst=00:00:00:00:00:02 actions=output:2
 cookie=0x0, duration=36.636s, table=1, n_packets=1517, n_bytes=148456, priority=100,dl_dst=00:00:00:00:00:03 actions=output:3
 cookie=0x0, duration=36.603s, table=1, n_packets=4, n_bytes=168, priority=100,dl_dst=ff:ff:ff:ff:ff:ff actions=FLOOD
 cookie=0x0, duration=36.570s, table=1, n_packets=3, n_bytes=222, priority=0 actions=drop


### OVS1
$ ovs-ofctl -O OpenFlow13 del-flows main-net 'table=1,dl_dst=00:00:00:00:00:03'
$ ovs-ofctl -O OpenFlow13 add-flow main-net 'table=1,priority=100,dl_dst=00:00:00:00:00:33,actions=output:eth2'


### PC1
$ wget -qO- -T 3 http://10.15.0.80/
SERVER OK


### PC2
$ ping -c 3 10.15.0.80
$ wget -qO- -T 3 http://10.15.0.80/
PING 10.15.0.80 (10.15.0.80): 56 data bytes
64 bytes from 10.15.0.80: seq=0 ttl=64 time=0.433 ms
64 bytes from 10.15.0.80: seq=1 ttl=64 time=0.128 ms
64 bytes from 10.15.0.80: seq=2 ttl=64 time=0.121 ms

--- 10.15.0.80 ping statistics ---
3 packets transmitted, 3 packets received, 0% packet loss
round-trip min/avg/max = 0.121/0.227/0.433 ms
wget: download timed out

## Pokretanje B (ponovo od cistog stanja)


## Korak 1 – Topologija

## Korak 2 – IP i MAC adrese, HTTP server

### PC1
$ ip link set dev eth0 address 00:00:00:00:00:01
$ ip addr flush dev eth0
$ ip addr add 10.15.0.11/24 dev eth0
$ ip addr show eth0
2: eth0@if347: <BROADCAST,MULTICAST,UP,LOWER_UP,M-DOWN> mtu 1500 qdisc noqueue state UP 
    link/ether 00:00:00:00:00:01 brd ff:ff:ff:ff:ff:ff
    inet 10.15.0.11/24 scope global eth0
       valid_lft forever preferred_lft forever


### PC2
$ ip link set dev eth0 address 00:00:00:00:00:02
$ ip addr flush dev eth0
$ ip addr add 10.15.0.12/24 dev eth0
$ ip addr show eth0
2: eth0@if348: <BROADCAST,MULTICAST,UP,LOWER_UP,M-DOWN> mtu 1500 qdisc noqueue state UP 
    link/ether 00:00:00:00:00:02 brd ff:ff:ff:ff:ff:ff
    inet 10.15.0.12/24 scope global eth0
       valid_lft forever preferred_lft forever


### SRV
$ ip link set dev eth0 address 00:00:00:00:00:03
$ ip addr flush dev eth0
$ ip addr add 10.15.0.80/24 dev eth0
$ cat > /srv.sh <<'X'
$ #!/bin/sh
$ while read l; do [ "$l" = "$(printf '\r')" ] && break; done
$ printf 'HTTP/1.0 200 OK\r\nContent-Type: text/plain\r\n\r\nSERVER OK\n'
$ X
$ chmod +x /srv.sh
$ nc -lk -p 80 -e /srv.sh &
$ ip addr show eth0
2: eth0@if349: <BROADCAST,MULTICAST,UP,LOWER_UP,M-DOWN> mtu 1500 qdisc noqueue state UP 
    link/ether 00:00:00:00:00:03 brd ff:ff:ff:ff:ff:ff
    inet 10.15.0.80/24 scope global eth0
       valid_lft forever preferred_lft forever


## Korak 3 – Bridge, OpenFlow 1.3 i prazna tabela pravila

### OVS1
$ ovs-vsctl del-port br0 eth0
$ ovs-vsctl del-port br0 eth1
$ ovs-vsctl del-port br0 eth2
$ ovs-vsctl add-br main-net
$ ovs-vsctl add-port main-net eth0
$ ovs-vsctl add-port main-net eth1
$ ovs-vsctl add-port main-net eth2
$ ovs-vsctl set bridge main-net protocols=OpenFlow13
$ ovs-ofctl -O OpenFlow13 del-flows main-net
$ ovs-ofctl -O OpenFlow13 dump-flows main-net
OFPST_FLOW reply (OF1.3) (xid=0x2):


### PC1
$ ping -c 3 -W 1 10.15.0.80
PING 10.15.0.80 (10.15.0.80): 56 data bytes

--- 10.15.0.80 ping statistics ---
3 packets transmitted, 0 packets received, 100% packet loss


## Korak 4 – Tabela 1: prosleđivanje po MAC adresi

### OVS1
$ ovs-ofctl -O OpenFlow13 add-flow main-net 'table=1,priority=100,dl_dst=00:00:00:00:00:01,actions=output:eth0'
$ ovs-ofctl -O OpenFlow13 add-flow main-net 'table=1,priority=100,dl_dst=00:00:00:00:00:02,actions=output:eth1'
$ ovs-ofctl -O OpenFlow13 add-flow main-net 'table=1,priority=100,dl_dst=00:00:00:00:00:03,actions=output:eth2'
$ ovs-ofctl -O OpenFlow13 add-flow main-net 'table=1,priority=100,dl_dst=ff:ff:ff:ff:ff:ff,actions=flood'
$ ovs-ofctl -O OpenFlow13 add-flow main-net 'table=1,priority=0,actions=drop'
$ ovs-ofctl -O OpenFlow13 dump-flows main-net
OFPST_FLOW reply (OF1.3) (xid=0x2):
 cookie=0x0, duration=0.136s, table=1, n_packets=0, n_bytes=0, priority=100,dl_dst=00:00:00:00:00:01 actions=output:1
 cookie=0x0, duration=0.103s, table=1, n_packets=0, n_bytes=0, priority=100,dl_dst=00:00:00:00:00:02 actions=output:2
 cookie=0x0, duration=0.070s, table=1, n_packets=0, n_bytes=0, priority=100,dl_dst=00:00:00:00:00:03 actions=output:3
 cookie=0x0, duration=0.036s, table=1, n_packets=0, n_bytes=0, priority=100,dl_dst=ff:ff:ff:ff:ff:ff actions=FLOOD
 cookie=0x0, duration=0.002s, table=1, n_packets=0, n_bytes=0, priority=0 actions=drop


### PC1
$ ping -c 3 -W 1 10.15.0.80
PING 10.15.0.80 (10.15.0.80): 56 data bytes

--- 10.15.0.80 ping statistics ---
3 packets transmitted, 0 packets received, 100% packet loss


## Korak 5 – Tabela 0: ACL

### OVS1
$ ovs-ofctl -O OpenFlow13 add-flow main-net 'table=0,priority=100,arp,actions=goto_table:1'
$ ovs-ofctl -O OpenFlow13 add-flow main-net 'table=0,priority=100,icmp,actions=goto_table:1'
$ ovs-ofctl -O OpenFlow13 add-flow main-net 'table=0,priority=100,tcp,in_port=eth0,nw_dst=10.15.0.80,tp_dst=80,actions=goto_table:1'
$ ovs-ofctl -O OpenFlow13 add-flow main-net 'table=0,priority=100,tcp,in_port=eth2,nw_src=10.15.0.80,tp_src=80,actions=goto_table:1'
$ ovs-ofctl -O OpenFlow13 add-flow main-net 'table=0,priority=50,tcp,tp_dst=80,actions=drop'
$ ovs-ofctl -O OpenFlow13 add-flow main-net 'table=0,priority=0,actions=drop'


### PC1
$ ping -c 3 10.15.0.12
$ ping -c 3 10.15.0.80
$ wget -qO- -T 3 http://10.15.0.80/
PING 10.15.0.12 (10.15.0.12): 56 data bytes
64 bytes from 10.15.0.12: seq=0 ttl=64 time=0.818 ms
64 bytes from 10.15.0.12: seq=1 ttl=64 time=0.093 ms
64 bytes from 10.15.0.12: seq=2 ttl=64 time=0.122 ms

--- 10.15.0.12 ping statistics ---
3 packets transmitted, 3 packets received, 0% packet loss
round-trip min/avg/max = 0.093/0.344/0.818 ms
PING 10.15.0.80 (10.15.0.80): 56 data bytes
64 bytes from 10.15.0.80: seq=0 ttl=64 time=0.656 ms
64 bytes from 10.15.0.80: seq=1 ttl=64 time=0.126 ms
64 bytes from 10.15.0.80: seq=2 ttl=64 time=0.112 ms

--- 10.15.0.80 ping statistics ---
3 packets transmitted, 3 packets received, 0% packet loss
round-trip min/avg/max = 0.112/0.298/0.656 ms
SERVER OK


### PC2
$ ping -c 3 10.15.0.80
$ wget -qO- -T 3 http://10.15.0.80/
PING 10.15.0.80 (10.15.0.80): 56 data bytes
64 bytes from 10.15.0.80: seq=0 ttl=64 time=0.571 ms
64 bytes from 10.15.0.80: seq=1 ttl=64 time=0.145 ms
64 bytes from 10.15.0.80: seq=2 ttl=64 time=0.099 ms

--- 10.15.0.80 ping statistics ---
3 packets transmitted, 3 packets received, 0% packet loss
round-trip min/avg/max = 0.099/0.271/0.571 ms
wget: download timed out


## Korak 6 – Brojači pravila

### OVS1
$ ovs-ofctl -O OpenFlow13 dump-flows main-net
OFPST_FLOW reply (OF1.3) (xid=0x2):
 cookie=0x0, duration=9.467s, table=0, n_packets=8, n_bytes=336, priority=100,arp actions=goto_table:1
 cookie=0x0, duration=9.434s, table=0, n_packets=18, n_bytes=1764, priority=100,icmp actions=goto_table:1
 cookie=0x0, duration=9.401s, table=0, n_packets=6, n_bytes=490, priority=100,tcp,in_port=1,nw_dst=10.15.0.80,tp_dst=80 actions=goto_table:1
 cookie=0x0, duration=9.368s, table=0, n_packets=6, n_bytes=459, priority=100,tcp,in_port=3,nw_src=10.15.0.80,tp_src=80 actions=goto_table:1
 cookie=0x0, duration=9.336s, table=0, n_packets=3, n_bytes=222, priority=50,tcp,tp_dst=80 actions=drop
 cookie=0x0, duration=9.305s, table=0, n_packets=3, n_bytes=210, priority=0 actions=drop
 cookie=0x0, duration=12.825s, table=1, n_packets=15, n_bytes=1173, priority=100,dl_dst=00:00:00:00:00:01 actions=output:1
 cookie=0x0, duration=12.792s, table=1, n_packets=8, n_bytes=672, priority=100,dl_dst=00:00:00:00:00:02 actions=output:2
 cookie=0x0, duration=12.759s, table=1, n_packets=12, n_bytes=1078, priority=100,dl_dst=00:00:00:00:00:03 actions=output:3
 cookie=0x0, duration=12.725s, table=1, n_packets=3, n_bytes=126, priority=100,dl_dst=ff:ff:ff:ff:ff:ff actions=FLOOD
 cookie=0x0, duration=12.691s, table=1, n_packets=0, n_bytes=0, priority=0 actions=drop


## Korak 7 – Meter i pravilo koje ga koristi

### OVS1
$ ovs-ofctl -O OpenFlow13 add-meter main-net 'meter=1,pktps,band=type=drop,rate=5'
$ ovs-ofctl -O OpenFlow13 dump-meters main-net
$ ovs-ofctl -O OpenFlow13 add-flow main-net 'table=0,priority=200,icmp,in_port=eth1,actions=meter:1,goto_table:1'
$ ovs-ofctl -O OpenFlow13 dump-flows main-net table=0
OFPST_METER_CONFIG reply (OF1.3) (xid=0x2):
meter=1 pktps bands=
type=drop rate=5
OFPST_FLOW reply (OF1.3) (xid=0x6):
 cookie=0x0, duration=0.031s, table=0, n_packets=0, n_bytes=0, priority=200,icmp,in_port=2 actions=meter:1,goto_table:1
 cookie=0x0, duration=9.577s, table=0, n_packets=18, n_bytes=1764, priority=100,icmp actions=goto_table:1
 cookie=0x0, duration=9.610s, table=0, n_packets=9, n_bytes=378, priority=100,arp actions=goto_table:1
 cookie=0x0, duration=9.511s, table=0, n_packets=6, n_bytes=459, priority=100,tcp,in_port=3,nw_src=10.15.0.80,tp_src=80 actions=goto_table:1
 cookie=0x0, duration=9.544s, table=0, n_packets=6, n_bytes=490, priority=100,tcp,in_port=1,nw_dst=10.15.0.80,tp_dst=80 actions=goto_table:1
 cookie=0x0, duration=9.479s, table=0, n_packets=3, n_bytes=222, priority=50,tcp,tp_dst=80 actions=drop
 cookie=0x0, duration=9.448s, table=0, n_packets=3, n_bytes=210, priority=0 actions=drop


## Korak 8 – Brzi ping i brojači meter-a

### PC2
$ ping -q -i 0.01 -c 500 -W 1 10.15.0.80
PING 10.15.0.80 (10.15.0.80): 56 data bytes

--- 10.15.0.80 ping statistics ---
500 packets transmitted, 30 packets received, 94% packet loss
round-trip min/avg/max = 0.038/0.061/0.093 ms


### PC1
$ ping -q -i 0.01 -c 500 -W 1 10.15.0.80
PING 10.15.0.80 (10.15.0.80): 56 data bytes

--- 10.15.0.80 ping statistics ---
500 packets transmitted, 500 packets received, 0% packet loss
round-trip min/avg/max = 0.023/0.049/0.397 ms


### OVS1
$ ovs-ofctl -O OpenFlow13 meter-stats main-net
$ ovs-ofctl -O OpenFlow13 dump-flows main-net table=0
OFPST_METER reply (OF1.3) (xid=0x2):
meter:1 flow_count:1 packet_in_count:500 byte_in_count:49000 duration:11.725s bands:
0: packet_count:470 byte_count:46060
OFPST_FLOW reply (OF1.3) (xid=0x6):
 cookie=0x0, duration=11.723s, table=0, n_packets=500, n_bytes=49000, priority=200,icmp,in_port=2 actions=meter:1,goto_table:1
 cookie=0x0, duration=21.269s, table=0, n_packets=1030, n_bytes=100940, priority=100,icmp actions=goto_table:1
 cookie=0x0, duration=21.302s, table=0, n_packets=10, n_bytes=420, priority=100,arp actions=goto_table:1
 cookie=0x0, duration=21.203s, table=0, n_packets=6, n_bytes=459, priority=100,tcp,in_port=3,nw_src=10.15.0.80,tp_src=80 actions=goto_table:1
 cookie=0x0, duration=21.236s, table=0, n_packets=6, n_bytes=490, priority=100,tcp,in_port=1,nw_dst=10.15.0.80,tp_dst=80 actions=goto_table:1
 cookie=0x0, duration=21.171s, table=0, n_packets=3, n_bytes=222, priority=50,tcp,tp_dst=80 actions=drop
 cookie=0x0, duration=21.140s, table=0, n_packets=3, n_bytes=210, priority=0 actions=drop


### PC2
$ ping -c 3 10.15.0.80
PING 10.15.0.80 (10.15.0.80): 56 data bytes
64 bytes from 10.15.0.80: seq=0 ttl=64 time=0.083 ms
64 bytes from 10.15.0.80: seq=1 ttl=64 time=0.111 ms
64 bytes from 10.15.0.80: seq=2 ttl=64 time=0.127 ms

--- 10.15.0.80 ping statistics ---
3 packets transmitted, 3 packets received, 0% packet loss
round-trip min/avg/max = 0.083/0.107/0.127 ms


## Korak 9 – Promena brzine meter-a

### OVS1
$ ovs-ofctl -O OpenFlow13 mod-meter main-net 'meter=1,pktps,band=type=drop,rate=50'
$ ovs-ofctl -O OpenFlow13 dump-meters main-net
OFPST_METER_CONFIG reply (OF1.3) (xid=0x2):
meter=1 pktps bands=
type=drop rate=50


### PC2
$ ping -q -i 0.01 -c 500 -W 1 10.15.0.80
PING 10.15.0.80 (10.15.0.80): 56 data bytes

--- 10.15.0.80 ping statistics ---
500 packets transmitted, 309 packets received, 38% packet loss
round-trip min/avg/max = 0.021/0.049/0.133 ms


## Korak 10 – Zamena MAC adrese servera menja samo tabelu 1

### SRV
$ ip link set dev eth0 address 00:00:00:00:00:33


### PC1
$ ip neigh flush dev eth0
10.15.0.12 lladdr 00:00:00:00:00:02 ref 1 used 0/0/0 probes 4 REACHABLE
10.15.0.80 lladdr 00:00:00:00:00:03 ref 1 used 0/0/0 probes 4 REACHABLE

*** Round 1, deleting 2 entries ***
*** Flush is complete after 1 round(s) ***


### PC2
$ ip neigh flush dev eth0
10.15.0.80 lladdr 00:00:00:00:00:03 used 0/0/0 probes 4 STALE
10.15.0.11 lladdr 00:00:00:00:00:01 ref 1 used 0/0/0 probes 1 REACHABLE

*** Round 1, deleting 2 entries ***
*** Flush is complete after 1 round(s) ***


### PC1
$ wget -qO- -T 3 http://10.15.0.80/
wget: download timed out


### OVS1
$ ovs-ofctl -O OpenFlow13 dump-flows main-net table=1
OFPST_FLOW reply (OF1.3) (xid=0x6):
 cookie=0x0, duration=36.687s, table=1, n_packets=516, n_bytes=50215, priority=100,dl_dst=00:00:00:00:00:01 actions=output:1
 cookie=0x0, duration=36.654s, table=1, n_packets=351, n_bytes=34230, priority=100,dl_dst=00:00:00:00:00:02 actions=output:2
 cookie=0x0, duration=36.621s, table=1, n_packets=1516, n_bytes=148414, priority=100,dl_dst=00:00:00:00:00:03 actions=output:3
 cookie=0x0, duration=36.587s, table=1, n_packets=4, n_bytes=168, priority=100,dl_dst=ff:ff:ff:ff:ff:ff actions=FLOOD
 cookie=0x0, duration=36.553s, table=1, n_packets=3, n_bytes=222, priority=0 actions=drop


### OVS1
$ ovs-ofctl -O OpenFlow13 del-flows main-net 'table=1,dl_dst=00:00:00:00:00:03'
$ ovs-ofctl -O OpenFlow13 add-flow main-net 'table=1,priority=100,dl_dst=00:00:00:00:00:33,actions=output:eth2'


### PC1
$ wget -qO- -T 3 http://10.15.0.80/
SERVER OK


### PC2
$ ping -c 3 10.15.0.80
$ wget -qO- -T 3 http://10.15.0.80/
PING 10.15.0.80 (10.15.0.80): 56 data bytes
64 bytes from 10.15.0.80: seq=0 ttl=64 time=0.448 ms
64 bytes from 10.15.0.80: seq=1 ttl=64 time=0.085 ms
64 bytes from 10.15.0.80: seq=2 ttl=64 time=0.086 ms

--- 10.15.0.80 ping statistics ---
3 packets transmitted, 3 packets received, 0% packet loss
round-trip min/avg/max = 0.085/0.206/0.448 ms
wget: download timed out
