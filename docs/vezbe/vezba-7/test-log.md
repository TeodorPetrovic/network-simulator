# Vežba 7 – test log (za održavanje, nije deo vežbe)

Testirano na pravim kontejnerima: `gns3/openvswitch` (OVS 2.17.12, bridge-evi br0..br3 sa `datapath_type=netdev`)
kao SW1 i SW2, i `alpine` kao PC1 i PC2. Imena kontejnera i mreža imaju prefiks `v7_`.

## Harness

- `src/test-setup.sh` – mreže `v7_n1` (SW1 eth0 – PC1 eth0), `v7_n2` (SW1 eth1 – PC2 eth0), `v7_n3` (SW1 eth2 – SW2 eth0).
  SW1 se pokreće sa `--entrypoint sleep` na n1, pa se n2 i n3 povezuju jedan po jedan (Docker broji ethN po redosledu
  povezivanja), zatim se `/etc/openvswitch/init.sh` pokreće ručno (stavlja sve ethN u br0, kao GNS3 pri startu).
  Docker IP adrese se brišu (`ip addr flush`). `ethtool -K ... tx off` nije bio potreban (samo ICMP i ARP).
- `src/run-test.py` – čita `<pre data-dev="...">` blokove iz `task/resenje.frag.html` i izvršava ih tačno kako su
  zapisani, redom (`docker exec v7_swN bash -c`, `docker exec v7_pcN sh -c`); komande prelomljene sa `\` šalju se
  kao jedna komanda sa prelomom, kao kad se nalepe u konzolu. Tokom Koraka 4 i 8 tcpdump (`v7-tcpdump:local`
  = alpine + tcpdump) snima u mrežnom namespace-u SW2 eth0 (trunk kabl), PC1 eth0 i PC2 eth0 – ekvivalent
  Wireshark snimanja na kablu u GNS3. Posle svakog koraka automatske provere (PASS/FAIL).
- Dodatne negativne provere samo u harness-u (nisu u vežbi): `arping` iz pogrešnog VLAN-a (PC2 traži 10.10.10.1,
  PC1 traži 10.10.20.1) ne dobija odgovor, i ping PC1 -> PC2 ne radi dok nema pravila za rutiranje.
- `src/test-teardown.sh` – briše samo `v7_*` kontejnere i mreže, po tačnom imenu.

## Rezultat

Četiri pokretanja od čistog stanja (run1–run3 pre pisanja konačnog teksta, run4 nad konačnim fragmentom): svako 29/29 PASS.
Ishodi identični u svim pokretanjima; razlikuju se samo vremena i brojač `rx` porta eth0 na SW2 u Koraku 8
(+7 ili +8 na prijemu zbog IPv6 multicast okvira računara koje SW2 odbacuje; `tx` uvek +6, svako pravilo za rutiranje +3).

Provereno:
- bez rutera PC1 i PC2 ne komuniciraju (K3, K4, K6 negativno);
- ARP zahtev PC1 prolazi br0 -> patch (trunk) -> br2 -> eth2 i na trunk kablu ima oznaku `vlan 10`; PC2 -> `vlan 20`;
- na kablovima ka PC1/PC2 nema nijednog 802.1Q okvira, a PC1 ne vidi ništa iz VLAN-a 20 (i obrnuto) – oznake ne cure na access portove;
- ARP za oba gateway-a odgovara OpenFlow (ip neigh: 02:00:00:00:10:01 i 02:00:00:00:20:01); zahtev iz pogrešnog VLAN-a bez odgovora;
- posle pravila za rutiranje ping radi u oba smera sa `ttl=63`;
- na trunk kablu svaki echo request se vidi dvaput sa istim IP id: vlan 10 / ttl 64 (PC1 MAC -> GW10 MAC) ka SW2,
  pa vlan 20 / ttl 63 (GW20 MAC -> PC2 MAC) od SW2; odgovori obrnuto; SW2 eth0 i prima i šalje (tx +6).

Činjenice proverene usput (koriste se u uvodu):
- `output:eth0` za paket koji je ušao na eth0 se tiho odbacuje (brojač pravila raste, ping 100% loss); `in_port` i
  `output:in_port` rade isto (dump-flows ih prikazuje kao `IN_PORT`).
- `tag=99` ili `trunks=30` na SW2 eth0 ne menja ništa za OpenFlow pravila (ping i dalje radi) – VLAN podešavanja porta koristi samo NORMAL.
- `vlan_tci=0x1000/0x1000`, `vlan_tci=0x0000/0x1000`, `strip_vlan`, `mod_vlan_vid` na neoznačenom okviru – sintaksa prihvaćena.
- Komanda prelomljena sa `\` unutar dvostrukih navodnika, sa uvlačenjem nastavka sa 2 razmaka, radi u `sh` i `bash`
  (ovs-ofctl prihvata razmak posle zareza u listi akcija).
- Patch par između br0 i br2 radi bez greške jer su oba bridge-a `netdev` (Vežba 5: patch između netdev i system bridge-a ne radi).

## Pun izlaz poslednjeg pokretanja (run4, nad konačnim fragmentom)

```

##### Korak 2 – MAC adrese, IP adrese i podrazumevani izlaz
[pc1]$ ip link set eth0 down
(rc=0)
[pc1]$ ip link set eth0 address 02:00:00:00:10:11
(rc=0)
[pc1]$ ip link set eth0 up
(rc=0)
[pc1]$ ip addr add 10.10.10.11/24 dev eth0
(rc=0)
[pc1]$ ip route add default via 10.10.10.1
(rc=0)
[pc2]$ ip link set eth0 down
(rc=0)
[pc2]$ ip link set eth0 address 02:00:00:00:20:12
(rc=0)
[pc2]$ ip link set eth0 up
(rc=0)
[pc2]$ ip addr add 10.10.20.12/24 dev eth0
(rc=0)
[pc2]$ ip route add default via 10.10.20.1
(rc=0)

##### Korak 3 – bridge-evi i VLAN uloge portova na SW1
[sw1]$ ovs-vsctl show
699e1ff2-d5af-46ac-aa39-06dc1712c367
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
    Bridge br3
        datapath_type: netdev
        Port br3
            Interface br3
                type: internal
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
(rc=0)
[sw1]$ ovs-vsctl del-port br0 eth1
(rc=0)
[sw1]$ ovs-vsctl del-port br0 eth2
(rc=0)
[sw1]$ ovs-vsctl add-port br2 eth1
(rc=0)
[sw1]$ ovs-vsctl add-port br2 eth2
(rc=0)
[sw1]$ ovs-vsctl set port eth0 tag=10
(rc=0)
[sw1]$ ovs-vsctl set port eth1 tag=20
(rc=0)
[sw1]$ ovs-vsctl set port eth2 trunks=10,20
(rc=0)
[sw1]$ ovs-vsctl show
699e1ff2-d5af-46ac-aa39-06dc1712c367
    Bridge br1
        datapath_type: netdev
        Port br1
            Interface br1
                type: internal
    Bridge br2
        datapath_type: netdev
        Port eth2
            trunks: [10, 20]
            Interface eth2
        Port eth1
            tag: 20
            Interface eth1
        Port br2
            Interface br2
                type: internal
    Bridge br3
        datapath_type: netdev
        Port br3
            Interface br3
                type: internal
    Bridge br0
        datapath_type: netdev
        Port eth0
            tag: 10
            Interface eth0
        Port br0
            Interface br0
                type: internal
(rc=0)
[pc1]$ ping -c 2 -W 2 10.10.20.12
PING 10.10.20.12 (10.10.20.12): 56 data bytes

--- 10.10.20.12 ping statistics ---
2 packets transmitted, 0 packets received, 100% packet loss
(rc=1)
CHECK PASS: K3 PC1->PC2 ping fails (no router, different VLAN) 
CHECK PASS: K3 tags/trunks set 

##### Korak 4 – patch portovi kao trunk između br0 i br2
[sw1]$ ovs-vsctl add-port br0 br0-br2 trunks=10,20 \
  -- set interface br0-br2 type=patch options:peer=br2-br0
(rc=0)
[sw1]$ ovs-vsctl add-port br2 br2-br0 trunks=10,20 \
  -- set interface br2-br0 type=patch options:peer=br0-br2
(rc=0)
[sw1]$ ovs-vsctl show
699e1ff2-d5af-46ac-aa39-06dc1712c367
    Bridge br1
        datapath_type: netdev
        Port br1
            Interface br1
                type: internal
    Bridge br2
        datapath_type: netdev
        Port br2-br0
            trunks: [10, 20]
            Interface br2-br0
                type: patch
                options: {peer=br0-br2}
        Port eth2
            trunks: [10, 20]
            Interface eth2
        Port eth1
            tag: 20
            Interface eth1
        Port br2
            Interface br2
                type: internal
    Bridge br3
        datapath_type: netdev
        Port br3
            Interface br3
                type: internal
    Bridge br0
        datapath_type: netdev
        Port eth0
            tag: 10
            Interface eth0
        Port br0-br2
            trunks: [10, 20]
            Interface br0-br2
                type: patch
                options: {peer=br2-br0}
        Port br0
            Interface br0
                type: internal
(rc=0)
[pc1]$ ping -c 2 -W 2 10.10.20.12
PING 10.10.20.12 (10.10.20.12): 56 data bytes

--- 10.10.20.12 ping statistics ---
2 packets transmitted, 0 packets received, 100% packet loss
(rc=1)
[pc2]$ ping -c 2 -W 2 10.10.10.11
PING 10.10.10.11 (10.10.10.11): 56 data bytes

--- 10.10.10.11 ping statistics ---
2 packets transmitted, 0 packets received, 100% packet loss
(rc=1)
----- capture K4 sw2eth0 (6 frames)
  11:04:22.224924 02:00:00:00:10:11 > ff:ff:ff:ff:ff:ff, ethertype 802.1Q (0x8100), length 46: vlan 10, p 0, ethertype ARP (0x0806), Ethernet (len 6), IPv4 (len 4), Request who-has 10.10.10.1 tell 10.10.10.11, length 28
  11:04:23.277861 02:00:00:00:10:11 > ff:ff:ff:ff:ff:ff, ethertype 802.1Q (0x8100), length 46: vlan 10, p 0, ethertype ARP (0x0806), Ethernet (len 6), IPv4 (len 4), Request who-has 10.10.10.1 tell 10.10.10.11, length 28
  11:04:24.301568 02:00:00:00:10:11 > ff:ff:ff:ff:ff:ff, ethertype 802.1Q (0x8100), length 46: vlan 10, p 0, ethertype ARP (0x0806), Ethernet (len 6), IPv4 (len 4), Request who-has 10.10.10.1 tell 10.10.10.11, length 28
  11:04:25.287858 02:00:00:00:20:12 > ff:ff:ff:ff:ff:ff, ethertype 802.1Q (0x8100), length 46: vlan 20, p 0, ethertype ARP (0x0806), Ethernet (len 6), IPv4 (len 4), Request who-has 10.10.20.1 tell 10.10.20.12, length 28
  11:04:26.349775 02:00:00:00:20:12 > ff:ff:ff:ff:ff:ff, ethertype 802.1Q (0x8100), length 46: vlan 20, p 0, ethertype ARP (0x0806), Ethernet (len 6), IPv4 (len 4), Request who-has 10.10.20.1 tell 10.10.20.12, length 28
  11:04:27.374026 02:00:00:00:20:12 > ff:ff:ff:ff:ff:ff, ethertype 802.1Q (0x8100), length 46: vlan 20, p 0, ethertype ARP (0x0806), Ethernet (len 6), IPv4 (len 4), Request who-has 10.10.20.1 tell 10.10.20.12, length 28
----- capture K4 pc1 (3 frames)
  11:04:22.224631 02:00:00:00:10:11 > ff:ff:ff:ff:ff:ff, ethertype ARP (0x0806), length 42: Ethernet (len 6), IPv4 (len 4), Request who-has 10.10.10.1 tell 10.10.10.11, length 28
  11:04:23.277485 02:00:00:00:10:11 > ff:ff:ff:ff:ff:ff, ethertype ARP (0x0806), length 42: Ethernet (len 6), IPv4 (len 4), Request who-has 10.10.10.1 tell 10.10.10.11, length 28
  11:04:24.301067 02:00:00:00:10:11 > ff:ff:ff:ff:ff:ff, ethertype ARP (0x0806), length 42: Ethernet (len 6), IPv4 (len 4), Request who-has 10.10.10.1 tell 10.10.10.11, length 28
----- capture K4 pc2 (3 frames)
  11:04:25.287206 02:00:00:00:20:12 > ff:ff:ff:ff:ff:ff, ethertype ARP (0x0806), length 42: Ethernet (len 6), IPv4 (len 4), Request who-has 10.10.20.1 tell 10.10.20.12, length 28
  11:04:26.349049 02:00:00:00:20:12 > ff:ff:ff:ff:ff:ff, ethertype ARP (0x0806), length 42: Ethernet (len 6), IPv4 (len 4), Request who-has 10.10.20.1 tell 10.10.20.12, length 28
  11:04:27.373476 02:00:00:00:20:12 > ff:ff:ff:ff:ff:ff, ethertype ARP (0x0806), length 42: Ethernet (len 6), IPv4 (len 4), Request who-has 10.10.20.1 tell 10.10.20.12, length 28
CHECK PASS: K4 patch ports present, no error 
CHECK PASS: K4 ping still fails 
CHECK PASS: K4 ping still fails 
CHECK PASS: K4 trunk: PC1 ARP arrives tagged vlan 10 (via patch) 
CHECK PASS: K4 trunk: PC2 ARP arrives tagged vlan 20 
CHECK PASS: K4 PC1 link: no 802.1Q frames 
CHECK PASS: K4 PC2 link: no 802.1Q frames 
CHECK PASS: K4 PC1 link: nothing from VLAN 20 
CHECK PASS: K4 PC2 link: nothing from VLAN 10 

##### Korak 5 – SW2 bez NORMAL pravila
[sw2]$ ovs-vsctl port-to-br eth0
br0
(rc=0)
[sw2]$ ovs-ofctl dump-flows br0
NXST_FLOW reply (xid=0x4):
 cookie=0x0, duration=14.153s, table=0, n_packets=24, n_bytes=1672, idle_age=1, priority=0 actions=NORMAL
(rc=0)
[sw2]$ ovs-ofctl del-flows br0
(rc=0)
[sw2]$ ovs-ofctl dump-flows br0
NXST_FLOW reply (xid=0x4):
(rc=0)
CHECK PASS: K5 NORMAL deleted 
CHECK PASS: K5 eth0 in br0 

##### Korak 6 – ARP responder za gateway svakog VLAN-a
[sw2]$ ovs-ofctl add-flow br0 "priority=200,in_port=eth0,dl_vlan=10,arp,arp_op=1,arp_tpa=10.10.10.1,\
  actions=move:NXM_OF_ETH_SRC[]->NXM_OF_ETH_DST[],mod_dl_src:02:00:00:00:10:01,\
  load:0x2->NXM_OF_ARP_OP[],move:NXM_NX_ARP_SHA[]->NXM_NX_ARP_THA[],\
  move:NXM_OF_ARP_SPA[]->NXM_OF_ARP_TPA[],load:0x020000001001->NXM_NX_ARP_SHA[],\
  load:0x0a0a0a01->NXM_OF_ARP_SPA[],in_port"
(rc=0)
[sw2]$ ovs-ofctl add-flow br0 "priority=200,in_port=eth0,dl_vlan=20,arp,arp_op=1,arp_tpa=10.10.20.1,\
  actions=move:NXM_OF_ETH_SRC[]->NXM_OF_ETH_DST[],mod_dl_src:02:00:00:00:20:01,\
  load:0x2->NXM_OF_ARP_OP[],move:NXM_NX_ARP_SHA[]->NXM_NX_ARP_THA[],\
  move:NXM_OF_ARP_SPA[]->NXM_OF_ARP_TPA[],load:0x020000002001->NXM_NX_ARP_SHA[],\
  load:0x0a0a1401->NXM_OF_ARP_SPA[],in_port"
(rc=0)
[pc1]$ ping -c 1 -W 1 10.10.10.1
PING 10.10.10.1 (10.10.10.1): 56 data bytes

--- 10.10.10.1 ping statistics ---
1 packets transmitted, 0 packets received, 100% packet loss
(rc=1)
[pc1]$ ip neigh show 10.10.10.1
10.10.10.1 dev eth0 lladdr 02:00:00:00:10:01 ref 1 used 0/0/0 probes 4 REACHABLE
(rc=0)
[pc2]$ ping -c 1 -W 1 10.10.20.1
PING 10.10.20.1 (10.10.20.1): 56 data bytes

--- 10.10.20.1 ping statistics ---
1 packets transmitted, 0 packets received, 100% packet loss
(rc=1)
[pc2]$ ip neigh show 10.10.20.1
10.10.20.1 dev eth0 lladdr 02:00:00:00:20:01 ref 1 used 0/0/0 probes 4 REACHABLE
(rc=0)
CHECK PASS: K6 PC1 learned GW10 MAC from OpenFlow 
CHECK PASS: K6 PC2 learned GW20 MAC from OpenFlow 
[pc2]$ arping -I eth0 -c 2 -w 3 10.10.10.1
ARPING 10.10.10.1 from 10.10.20.12 eth0
Sent 2 probe(s) (0 broadcast(s))
Received 0 response(s) (0 request(s), 0 broadcast(s))
(rc=1)
CHECK PASS: K6 NEG: ARP for 10.10.10.1 asked from VLAN 20 is NOT answered 
[pc1]$ arping -I eth0 -c 2 -w 3 10.10.20.1
ARPING 10.10.20.1 from 10.10.10.11 eth0
Sent 2 probe(s) (0 broadcast(s))
Received 0 response(s) (0 request(s), 0 broadcast(s))
(rc=1)
CHECK PASS: K6 NEG: ARP for 10.10.20.1 asked from VLAN 10 is NOT answered 
[pc1]$ ping -c 2 -W 2 10.10.20.12
PING 10.10.20.12 (10.10.20.12): 56 data bytes

--- 10.10.20.12 ping statistics ---
2 packets transmitted, 0 packets received, 100% packet loss
(rc=1)
CHECK PASS: K6 NEG: PC1->PC2 still fails without routing flows 

##### Korak 7 – rutiranje između VLAN-ova
[sw2]$ ovs-ofctl add-flow br0 "priority=100,in_port=eth0,dl_vlan=10,ip,nw_dst=10.10.20.0/24,\
  actions=mod_vlan_vid:20,mod_dl_src:02:00:00:00:20:01,mod_dl_dst:02:00:00:00:20:12,dec_ttl,in_port"
(rc=0)
[sw2]$ ovs-ofctl add-flow br0 "priority=100,in_port=eth0,dl_vlan=20,ip,nw_dst=10.10.10.0/24,\
  actions=mod_vlan_vid:10,mod_dl_src:02:00:00:00:10:01,mod_dl_dst:02:00:00:00:10:11,dec_ttl,in_port"
(rc=0)
[pc1]$ ping -c 2 -W 2 10.10.20.12
PING 10.10.20.12 (10.10.20.12): 56 data bytes
64 bytes from 10.10.20.12: seq=0 ttl=63 time=1.503 ms
64 bytes from 10.10.20.12: seq=1 ttl=63 time=1.518 ms

--- 10.10.20.12 ping statistics ---
2 packets transmitted, 2 packets received, 0% packet loss
round-trip min/avg/max = 1.503/1.510/1.518 ms
(rc=0)
[pc2]$ ping -c 2 -W 2 10.10.10.11
PING 10.10.10.11 (10.10.10.11): 56 data bytes
64 bytes from 10.10.10.11: seq=0 ttl=63 time=1.684 ms
64 bytes from 10.10.10.11: seq=1 ttl=63 time=2.285 ms

--- 10.10.10.11 ping statistics ---
2 packets transmitted, 2 packets received, 0% packet loss
round-trip min/avg/max = 1.684/1.984/2.285 ms
(rc=0)
CHECK PASS: K7 ping works, ttl=63 
CHECK PASS: K7 ping works, ttl=63 

##### Korak 8 – dokaz da saobraćaj ide preko SW2
[sw2]$ ovs-ofctl --names dump-ports br0 eth0
OFPST_PORT reply (xid=0x4): 1 ports
  port  eth0: rx pkts=49, bytes=3714, drop=0, errs=0, frame=0, over=0, crc=0
           tx pkts=21, bytes=1690, drop=0, errs=0, coll=0
(rc=0)
[sw2]$ ovs-ofctl --names dump-flows br0
 cookie=0x0, duration=13.770s, table=0, n_packets=1, n_bytes=46, priority=200,arp,in_port=eth0,dl_vlan=10,arp_tpa=10.10.10.1,arp_op=1 actions=move:NXM_OF_ETH_SRC[]->NXM_OF_ETH_DST[],mod_dl_src:02:00:00:00:10:01,load:0x2->NXM_OF_ARP_OP[],move:NXM_NX_ARP_SHA[]->NXM_NX_ARP_THA[],move:NXM_OF_ARP_SPA[]->NXM_OF_ARP_TPA[],load:0x20000001001->NXM_NX_ARP_SHA[],load:0xa0a0a01->NXM_OF_ARP_SPA[],IN_PORT
 cookie=0x0, duration=13.717s, table=0, n_packets=1, n_bytes=46, priority=200,arp,in_port=eth0,dl_vlan=20,arp_tpa=10.10.20.1,arp_op=1 actions=move:NXM_OF_ETH_SRC[]->NXM_OF_ETH_DST[],mod_dl_src:02:00:00:00:20:01,load:0x2->NXM_OF_ARP_OP[],move:NXM_NX_ARP_SHA[]->NXM_NX_ARP_THA[],move:NXM_OF_ARP_SPA[]->NXM_OF_ARP_TPA[],load:0x20000002001->NXM_NX_ARP_SHA[],load:0xa0a1401->NXM_OF_ARP_SPA[],IN_PORT
 cookie=0x0, duration=4.257s, table=0, n_packets=4, n_bytes=408, priority=100,ip,in_port=eth0,dl_vlan=10,nw_dst=10.10.20.0/24 actions=mod_vlan_vid:20,mod_dl_src:02:00:00:00:20:01,mod_dl_dst:02:00:00:00:20:12,dec_ttl,IN_PORT
 cookie=0x0, duration=4.204s, table=0, n_packets=4, n_bytes=408, priority=100,ip,in_port=eth0,dl_vlan=20,nw_dst=10.10.10.0/24 actions=mod_vlan_vid:10,mod_dl_src:02:00:00:00:10:01,mod_dl_dst:02:00:00:00:10:11,dec_ttl,IN_PORT
(rc=0)
[pc1]$ ping -c 3 -W 2 10.10.20.12
PING 10.10.20.12 (10.10.20.12): 56 data bytes
64 bytes from 10.10.20.12: seq=0 ttl=63 time=1.800 ms
64 bytes from 10.10.20.12: seq=1 ttl=63 time=1.943 ms
64 bytes from 10.10.20.12: seq=2 ttl=63 time=1.898 ms

--- 10.10.20.12 ping statistics ---
3 packets transmitted, 3 packets received, 0% packet loss
round-trip min/avg/max = 1.800/1.880/1.943 ms
(rc=0)
[sw2]$ ovs-ofctl --names dump-ports br0 eth0
OFPST_PORT reply (xid=0x4): 1 ports
  port  eth0: rx pkts=56, bytes=4372, drop=0, errs=0, frame=0, over=0, crc=0
           tx pkts=27, bytes=2302, drop=0, errs=0, coll=0
(rc=0)
[sw2]$ ovs-ofctl --names dump-flows br0
 cookie=0x0, duration=15.929s, table=0, n_packets=1, n_bytes=46, priority=200,arp,in_port=eth0,dl_vlan=10,arp_tpa=10.10.10.1,arp_op=1 actions=move:NXM_OF_ETH_SRC[]->NXM_OF_ETH_DST[],mod_dl_src:02:00:00:00:10:01,load:0x2->NXM_OF_ARP_OP[],move:NXM_NX_ARP_SHA[]->NXM_NX_ARP_THA[],move:NXM_OF_ARP_SPA[]->NXM_OF_ARP_TPA[],load:0x20000001001->NXM_NX_ARP_SHA[],load:0xa0a0a01->NXM_OF_ARP_SPA[],IN_PORT
 cookie=0x0, duration=15.876s, table=0, n_packets=1, n_bytes=46, priority=200,arp,in_port=eth0,dl_vlan=20,arp_tpa=10.10.20.1,arp_op=1 actions=move:NXM_OF_ETH_SRC[]->NXM_OF_ETH_DST[],mod_dl_src:02:00:00:00:20:01,load:0x2->NXM_OF_ARP_OP[],move:NXM_NX_ARP_SHA[]->NXM_NX_ARP_THA[],move:NXM_OF_ARP_SPA[]->NXM_OF_ARP_TPA[],load:0x20000002001->NXM_NX_ARP_SHA[],load:0xa0a1401->NXM_OF_ARP_SPA[],IN_PORT
 cookie=0x0, duration=6.416s, table=0, n_packets=7, n_bytes=714, priority=100,ip,in_port=eth0,dl_vlan=10,nw_dst=10.10.20.0/24 actions=mod_vlan_vid:20,mod_dl_src:02:00:00:00:20:01,mod_dl_dst:02:00:00:00:20:12,dec_ttl,IN_PORT
 cookie=0x0, duration=6.363s, table=0, n_packets=7, n_bytes=714, priority=100,ip,in_port=eth0,dl_vlan=20,nw_dst=10.10.10.0/24 actions=mod_vlan_vid:10,mod_dl_src:02:00:00:00:10:01,mod_dl_dst:02:00:00:00:10:11,dec_ttl,IN_PORT
(rc=0)
----- capture K8 sw2eth0 (12 frames)
  11:04:43.661098 02:00:00:00:10:11 > 02:00:00:00:10:01, ethertype 802.1Q (0x8100), length 102: vlan 10, p 0, ethertype IPv4 (0x0800), (tos 0x0, ttl 64, id 3376, offset 0, flags [DF], proto ICMP (1), length 84) | 10.10.10.11 > 10.10.20.12: ICMP echo request, id 90, seq 0, length 64
  11:04:43.661449 02:00:00:00:20:01 > 02:00:00:00:20:12, ethertype 802.1Q (0x8100), length 102: vlan 20, p 0, ethertype IPv4 (0x0800), (tos 0x0, ttl 63, id 3376, offset 0, flags [DF], proto ICMP (1), length 84) | 10.10.10.11 > 10.10.20.12: ICMP echo request, id 90, seq 0, length 64
  11:04:43.661870 02:00:00:00:20:12 > 02:00:00:00:20:01, ethertype 802.1Q (0x8100), length 102: vlan 20, p 0, ethertype IPv4 (0x0800), (tos 0x0, ttl 64, id 20725, offset 0, flags [none], proto ICMP (1), length 84) | 10.10.20.12 > 10.10.10.11: ICMP echo reply, id 90, seq 0, length 64
  11:04:43.662072 02:00:00:00:10:01 > 02:00:00:00:10:11, ethertype 802.1Q (0x8100), length 102: vlan 10, p 0, ethertype IPv4 (0x0800), (tos 0x0, ttl 63, id 20725, offset 0, flags [none], proto ICMP (1), length 84) | 10.10.20.12 > 10.10.10.11: ICMP echo reply, id 90, seq 0, length 64
  11:04:44.661269 02:00:00:00:10:11 > 02:00:00:00:10:01, ethertype 802.1Q (0x8100), length 102: vlan 10, p 0, ethertype IPv4 (0x0800), (tos 0x0, ttl 64, id 3474, offset 0, flags [DF], proto ICMP (1), length 84) | 10.10.10.11 > 10.10.20.12: ICMP echo request, id 90, seq 1, length 64
  11:04:44.661537 02:00:00:00:20:01 > 02:00:00:00:20:12, ethertype 802.1Q (0x8100), length 102: vlan 20, p 0, ethertype IPv4 (0x0800), (tos 0x0, ttl 63, id 3474, offset 0, flags [DF], proto ICMP (1), length 84) | 10.10.10.11 > 10.10.20.12: ICMP echo request, id 90, seq 1, length 64
  11:04:44.661902 02:00:00:00:20:12 > 02:00:00:00:20:01, ethertype 802.1Q (0x8100), length 102: vlan 20, p 0, ethertype IPv4 (0x0800), (tos 0x0, ttl 64, id 21594, offset 0, flags [none], proto ICMP (1), length 84) | 10.10.20.12 > 10.10.10.11: ICMP echo reply, id 90, seq 1, length 64
  11:04:44.662377 02:00:00:00:10:01 > 02:00:00:00:10:11, ethertype 802.1Q (0x8100), length 102: vlan 10, p 0, ethertype IPv4 (0x0800), (tos 0x0, ttl 63, id 21594, offset 0, flags [none], proto ICMP (1), length 84) | 10.10.20.12 > 10.10.10.11: ICMP echo reply, id 90, seq 1, length 64
  11:04:45.662598 02:00:00:00:10:11 > 02:00:00:00:10:01, ethertype 802.1Q (0x8100), length 102: vlan 10, p 0, ethertype IPv4 (0x0800), (tos 0x0, ttl 64, id 4132, offset 0, flags [DF], proto ICMP (1), length 84) | 10.10.10.11 > 10.10.20.12: ICMP echo request, id 90, seq 2, length 64
  11:04:45.663085 02:00:00:00:20:01 > 02:00:00:00:20:12, ethertype 802.1Q (0x8100), length 102: vlan 20, p 0, ethertype IPv4 (0x0800), (tos 0x0, ttl 63, id 4132, offset 0, flags [DF], proto ICMP (1), length 84) | 10.10.10.11 > 10.10.20.12: ICMP echo request, id 90, seq 2, length 64
  11:04:45.663475 02:00:00:00:20:12 > 02:00:00:00:20:01, ethertype 802.1Q (0x8100), length 102: vlan 20, p 0, ethertype IPv4 (0x0800), (tos 0x0, ttl 64, id 21842, offset 0, flags [none], proto ICMP (1), length 84) | 10.10.20.12 > 10.10.10.11: ICMP echo reply, id 90, seq 2, length 64
  11:04:45.663669 02:00:00:00:10:01 > 02:00:00:00:10:11, ethertype 802.1Q (0x8100), length 102: vlan 10, p 0, ethertype IPv4 (0x0800), (tos 0x0, ttl 63, id 21842, offset 0, flags [none], proto ICMP (1), length 84) | 10.10.20.12 > 10.10.10.11: ICMP echo reply, id 90, seq 2, length 64
----- capture K8 pc1 (6 frames)
  11:04:43.660578 02:00:00:00:10:11 > 02:00:00:00:10:01, ethertype IPv4 (0x0800), length 98: (tos 0x0, ttl 64, id 3376, offset 0, flags [DF], proto ICMP (1), length 84) | 10.10.10.11 > 10.10.20.12: ICMP echo request, id 90, seq 0, length 64
  11:04:43.662303 02:00:00:00:10:01 > 02:00:00:00:10:11, ethertype IPv4 (0x0800), length 98: (tos 0x0, ttl 63, id 20725, offset 0, flags [none], proto ICMP (1), length 84) | 10.10.20.12 > 10.10.10.11: ICMP echo reply, id 90, seq 0, length 64
  11:04:44.660997 02:00:00:00:10:11 > 02:00:00:00:10:01, ethertype IPv4 (0x0800), length 98: (tos 0x0, ttl 64, id 3474, offset 0, flags [DF], proto ICMP (1), length 84) | 10.10.10.11 > 10.10.20.12: ICMP echo request, id 90, seq 1, length 64
  11:04:44.662844 02:00:00:00:10:01 > 02:00:00:00:10:11, ethertype IPv4 (0x0800), length 98: (tos 0x0, ttl 63, id 21594, offset 0, flags [none], proto ICMP (1), length 84) | 10.10.20.12 > 10.10.10.11: ICMP echo reply, id 90, seq 1, length 64
  11:04:45.662115 02:00:00:00:10:11 > 02:00:00:00:10:01, ethertype IPv4 (0x0800), length 98: (tos 0x0, ttl 64, id 4132, offset 0, flags [DF], proto ICMP (1), length 84) | 10.10.10.11 > 10.10.20.12: ICMP echo request, id 90, seq 2, length 64
  11:04:45.663898 02:00:00:00:10:01 > 02:00:00:00:10:11, ethertype IPv4 (0x0800), length 98: (tos 0x0, ttl 63, id 21842, offset 0, flags [none], proto ICMP (1), length 84) | 10.10.20.12 > 10.10.10.11: ICMP echo reply, id 90, seq 2, length 64
----- capture K8 pc2 (6 frames)
  11:04:43.661721 02:00:00:00:20:01 > 02:00:00:00:20:12, ethertype IPv4 (0x0800), length 98: (tos 0x0, ttl 63, id 3376, offset 0, flags [DF], proto ICMP (1), length 84) | 10.10.10.11 > 10.10.20.12: ICMP echo request, id 90, seq 0, length 64
  11:04:43.661763 02:00:00:00:20:12 > 02:00:00:00:20:01, ethertype IPv4 (0x0800), length 98: (tos 0x0, ttl 64, id 20725, offset 0, flags [none], proto ICMP (1), length 84) | 10.10.20.12 > 10.10.10.11: ICMP echo reply, id 90, seq 0, length 64
  11:04:44.661788 02:00:00:00:20:01 > 02:00:00:00:20:12, ethertype IPv4 (0x0800), length 98: (tos 0x0, ttl 63, id 3474, offset 0, flags [DF], proto ICMP (1), length 84) | 10.10.10.11 > 10.10.20.12: ICMP echo request, id 90, seq 1, length 64
  11:04:44.661810 02:00:00:00:20:12 > 02:00:00:00:20:01, ethertype IPv4 (0x0800), length 98: (tos 0x0, ttl 64, id 21594, offset 0, flags [none], proto ICMP (1), length 84) | 10.10.20.12 > 10.10.10.11: ICMP echo reply, id 90, seq 1, length 64
  11:04:45.663344 02:00:00:00:20:01 > 02:00:00:00:20:12, ethertype IPv4 (0x0800), length 98: (tos 0x0, ttl 63, id 4132, offset 0, flags [DF], proto ICMP (1), length 84) | 10.10.10.11 > 10.10.20.12: ICMP echo request, id 90, seq 2, length 64
  11:04:45.663370 02:00:00:00:20:12 > 02:00:00:00:20:01, ethertype IPv4 (0x0800), length 98: (tos 0x0, ttl 64, id 21842, offset 0, flags [none], proto ICMP (1), length 84) | 10.10.20.12 > 10.10.10.11: ICMP echo reply, id 90, seq 2, length 64
port eth0 delta rx 7 tx 6 | flow deltas {'priority=200,arp,in_port=eth0,dl_vlan=10,arp_tpa=10.10.10.1,arp_op=1': 0, 'priority=200,arp,in_port=eth0,dl_vlan=20,arp_tpa=10.10.20.1,arp_op=1': 0, 'priority=100,ip,in_port=eth0,dl_vlan=10,nw_dst=10.10.20.0/24': 3, 'priority=100,ip,in_port=eth0,dl_vlan=20,nw_dst=10.10.10.0/24': 3}
CHECK PASS: K8 eth0 rx and tx both grew by >= 6 rx+7 tx+6
CHECK PASS: K8 routing flow counters +3 each [3] [3]
CHECK PASS: K8 ping ok 
CHECK PASS: K8 trunk: every request seen twice: vlan10/ttl64 in, vlan20/ttl63 out 3 ids
CHECK PASS: K8 trunk: replies vlan20/ttl64 in, vlan10/ttl63 out 
CHECK PASS: K8 PC1 link: no 802.1Q 
CHECK PASS: K8 PC2 link: no 802.1Q 
CHECK PASS: K8 PC2 sees request from GW20 MAC with ttl 63 
CHECK PASS: K8 PC1 sends request to GW10 MAC with ttl 64 

===== SUMMARY
PASS K3 PC1->PC2 ping fails (no router, different VLAN)
PASS K3 tags/trunks set
PASS K4 patch ports present, no error
PASS K4 ping still fails
PASS K4 ping still fails
PASS K4 trunk: PC1 ARP arrives tagged vlan 10 (via patch)
PASS K4 trunk: PC2 ARP arrives tagged vlan 20
PASS K4 PC1 link: no 802.1Q frames
PASS K4 PC2 link: no 802.1Q frames
PASS K4 PC1 link: nothing from VLAN 20
PASS K4 PC2 link: nothing from VLAN 10
PASS K5 NORMAL deleted
PASS K5 eth0 in br0
PASS K6 PC1 learned GW10 MAC from OpenFlow
PASS K6 PC2 learned GW20 MAC from OpenFlow
PASS K6 NEG: ARP for 10.10.10.1 asked from VLAN 20 is NOT answered
PASS K6 NEG: ARP for 10.10.20.1 asked from VLAN 10 is NOT answered
PASS K6 NEG: PC1->PC2 still fails without routing flows
PASS K7 ping works, ttl=63
PASS K7 ping works, ttl=63
PASS K8 eth0 rx and tx both grew by >= 6
PASS K8 routing flow counters +3 each
PASS K8 ping ok
PASS K8 trunk: every request seen twice: vlan10/ttl64 in, vlan20/ttl63 out
PASS K8 trunk: replies vlan20/ttl64 in, vlan10/ttl63 out
PASS K8 PC1 link: no 802.1Q
PASS K8 PC2 link: no 802.1Q
PASS K8 PC2 sees request from GW20 MAC with ttl 63
PASS K8 PC1 sends request to GW10 MAC with ttl 64
29/29 PASS
```
