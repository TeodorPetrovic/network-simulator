# Vežba 7 – test log (za održavanje, nije deo vežbe)

**Važno: MikroTik nije testiran.** MikroTik slika nije dostupna, pa je ruter u testu EMULIRAN Linux kontejnerom
`v7_router` (alpine + iproute2): VLAN podinterfejsi `eth0.10` (10.10.10.1/24) i `eth0.20` (10.10.20.1/24) na jednom
kablu, `net.ipv4.ip_forward=1`. RouterOS komande iz Koraka 4 rešenja (`/interface vlan add ...`, `/ip address add ...`)
i primer komandi u zadatku (VLAN 5) **nisu izvršene ni proverene**. Izlazi pingova ka gateway-u (`ttl=64`) u rešenju
potiču od Linux rutera. Sve ostalo (SW1, SW2, VPC1, VPC2) testirano je na pravim uređajima.

Uređaji: `gns3/openvswitch` (OVS 2.17, bridge-evi br0..br3 sa `datapath_type=netdev`) kao SW1 i SW2, pravi `vpcs`
0.8.5 iz simulatora (`-p <konzola> -m <mac> -i 1 -F -R -s <lport> -c <rport> -t 127.0.0.1`, preko `ubridge`-a na eth0
kontejnera) kao VPC1 i VPC2. Bez DHCP-a (sve adrese statičke). Imena kontejnera i mreža imaju prefiks `v7_`.

## Harness

- `src/test-setup.sh` – mreže (kablovi) `v7_n1` (SW1 eth0 – VPC1), `v7_n2` (SW1 eth1 – VPC2), `v7_n3`
  (SW1 eth2 – SW2 eth0), `v7_n4` (SW2 eth1 – ruter eth0 = MikroTik ether1). Kontejneri se povezuju jedan po jedan
  (Docker broji ethN po redosledu), `/etc/openvswitch/init.sh` se pokreće ručno (sve ethN u br0, kao GNS3).
- `src/run-test.py` – izvršava `<pre data-dev="...">` blokove iz `task/resenje.frag.html` tačno kako su zapisani:
  sw1/sw2 kroz `docker exec`, vpc1/vpc2 kroz telnet konzolu VPCS-a (konzola ostaje otvorena do kraja – zatvaranje
  konzole resetuje VPCS i briše IP adresu). Blok `mikrotik` se NE izvršava: umesto njega se na `v7_router` izvršava
  Linux ekvivalent (`ROUTER_EMU`). Tokom Koraka 6 tcpdump (`v7-tcpdump:local`, deli netns) snima kablove SW1–SW2
  (SW2 eth0), SW2–ruter (SW2 eth1), VPC1 i VPC2.
- Harness-only provere (nisu u vežbi): `show` na VPC-ovima (MAC za proveru snimaka); negativni test VLAN 30:
  kontejner `v7_inj` deli netns sa SW1 i pravi `eth2.30` (10.10.30.2/24) direktno na kablu ka SW2, ruter dobija
  `eth0.30` (10.10.30.1/24); ping u oba smera mora da padne, okviri VLAN 30 moraju da stignu do SW2 (eth0 odnosno
  eth1) a da se ne pojave na drugom portu, i brojač pravila `priority=0 actions=drop` mora da raste.
- Samo-provera harness-a: `V7_LEAK30=1` dodaje na SW2 pravilo koje propušta VLAN 30 – tada 5 NEG provera pada
  (25/30), što potvrđuje da negativni test zaista hvata curenje.
- `src/test-teardown.sh` – briše samo `v7_*` kontejnere i mreže, po tačnom imenu.

Pokretanje (u `src/`):

    ./test-teardown.sh; ./test-setup.sh && python3 run-test.py; ./test-teardown.sh
    V7_M1=21 V7_M2=179 ./test-setup.sh ...      # druge MAC adrese VPC-ova

## Rezultat

Tri pokretanja od čistog stanja nad konačnim fragmentom, svako 30/30 PASS:

| run | MAC VPC1 / VPC2 | rezultat |
|-----|-----------------|----------|
| 1 | `00:50:79:66:68:1c` / `00:50:79:66:68:a7` | 30/30 PASS (pun izlaz ispod) |
| 2 | `00:50:79:66:68:15` / `00:50:79:66:68:b3` | 30/30 PASS |
| 3 | `00:50:79:66:68:c0` / `00:50:79:66:68:aa` | 30/30 PASS |

Ishodi identični; razlikuju se samo vremena i MAC adrese.

Provereno (SW1/SW2/VPCS stvarno, ruter emuliran):
- SW1: `tag: 10`, `tag: 20`, `trunks: [10, 20]` na eth2 i oba patch porta, patch par bez greške;
- SW2 posle `del-flows` (bez NORMAL), sa ruterom već podešenim: VPC1 -> VPC2 ne radi (`host (10.10.10.1) not reachable`);
- SW2 sa tačno 5 pravila (4 x `in_port`/`dl_vlan` -> `output`, plus `priority=0 actions=drop`), bez NORMAL: ping VPC-ova
  ka svom gateway-u radi (`ttl=64`), VPC1 <-> VPC2 radi u oba smera sa `ttl=63`; sva 4 pravila za prosleđivanje
  imaju porast brojača;
- kabl SW1–SW2 i kabl SW2–ruter: svaki okvir ima 802.1Q oznaku 10 ili 20; echo request VPC1 -> VPC2 se na oba kabla
  vidi kao vlan 10 / ttl 64 (VPC1 MAC -> MAC rutera) i vlan 20 / ttl 63 (MAC rutera -> VPC2 MAC);
- kablovi ka VPC1 i VPC2: nijedan 802.1Q okvir; VPC2 prima zahtev od MAC adrese rutera sa ttl 63;
- VLAN 30 (nije na listi) SW2 odbacuje u oba smera; bez pravila za VLAN 30 ping na VLAN 30 ne radi, a uz `V7_LEAK30=1`
  radi (provera da test hvata curenje).

Nije provereno: RouterOS sintaksa i ponašanje pravog MikroTik-a (Korak 4 rešenja i primer komandi u zadatku).

## Pun izlaz pokretanja 1

```

##### Korak 2 – IP adrese i podrazumevani izlaz na VPC-ovima
[VPC1> ip 10.10.10.11/24 10.10.10.1
Checking for duplicate address...
VPC1 : 10.10.10.11 255.255.255.0 gateway 10.10.10.1
(rc=0)
[VPC2> ip 10.10.20.12/24 10.10.20.1
Checking for duplicate address...
VPC2 : 10.10.20.12 255.255.255.0 gateway 10.10.20.1
(rc=0)
[VPC1> show
NAME   IP/MASK              GATEWAY           MAC                LPORT  RHOST:PORT
VPC1   10.10.10.11/24       10.10.10.1        00:50:79:66:68:1c  20000  127.0.0.1:20001
       fe80::250:79ff:fe66:681c/64
(rc=0)
CHECK PASS: K2 vpc1: IP and gateway set VPC1   10.10.10.11/24       10.10.10.1        00:50:79:66:68:1c
[VPC2> show
NAME   IP/MASK              GATEWAY           MAC                LPORT  RHOST:PORT
VPC2   10.10.20.12/24       10.10.20.1        00:50:79:66:68:a7  20000  127.0.0.1:20001
       fe80::250:79ff:fe66:68a7/64
(rc=0)
CHECK PASS: K2 vpc2: IP and gateway set VPC2   10.10.20.12/24       10.10.20.1        00:50:79:66:68:a7
MAC: {'router': '86:87:3c:21:3b:17', 'sw1eth2': 'ba:cb:76:42:64:28', 'vpc1': '00:50:79:66:68:1c', 'vpc2': '00:50:79:66:68:a7'}

##### Korak 3 – SW1: bridge-evi, access i trunk portovi, patch par
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
[sw1]$ ovs-vsctl add-port br0 br0-br2 trunks=10,20 \
  -- set interface br0-br2 type=patch options:peer=br2-br0
(rc=0)
[sw1]$ ovs-vsctl add-port br2 br2-br0 trunks=10,20 \
  -- set interface br2-br0 type=patch options:peer=br0-br2
(rc=0)
[sw1]$ ovs-vsctl show
fedd5f9e-2f61-4f8d-8016-d02bfdb9a651
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
    Bridge br2
        datapath_type: netdev
        Port eth2
            trunks: [10, 20]
            Interface eth2
        Port eth1
            tag: 20
            Interface eth1
        Port br2-br0
            trunks: [10, 20]
            Interface br2-br0
                type: patch
                options: {peer=br0-br2}
        Port br2
            Interface br2
                type: internal
    Bridge br1
        datapath_type: netdev
        Port br1
            Interface br1
                type: internal
(rc=0)
CHECK PASS: K3 tags/trunks set 
CHECK PASS: K3 patch ports present, no error 

##### Korak 4 – MikroTik: VLAN podinterfejsi (nije izvršeno u testu)
[MikroTik] RouterOS komande NISU izvrsene (nema MikroTik slike):
    /interface vlan add name=vlan10 vlan-id=10 interface=ether1
    /interface vlan add name=vlan20 vlan-id=20 interface=ether1
    /ip address add address=10.10.10.1/24 interface=vlan10
    /ip address add address=10.10.20.1/24 interface=vlan20
    /interface vlan print
    /ip address print
[MikroTik] EMULACIJA: Linux ruter v7_router izvrsava ekvivalent:
[router]# ip link add link eth0 name eth0.10 type vlan id 10

(rc=0)
[router]# ip link add link eth0 name eth0.20 type vlan id 20

(rc=0)
[router]# ip addr add 10.10.10.1/24 dev eth0.10

(rc=0)
[router]# ip addr add 10.10.20.1/24 dev eth0.20

(rc=0)
[router]# ip link set eth0.10 up

(rc=0)
[router]# ip link set eth0.20 up

(rc=0)
[router]# sysctl net.ipv4.ip_forward
net.ipv4.ip_forward = 1
(rc=0)
[router]# ip -br addr
lo               UNKNOWN        127.0.0.1/8 
eth0@if1120      UP             
eth0.10@eth0     UP             10.10.10.1/24 
eth0.20@eth0     UP             10.10.20.1/24
(rc=0)
CHECK PASS: K4 (EMULATED router) sub-interfaces 10.10.10.1 / 10.10.20.1 lo               UNKNOWN        127.0.0.1/8 
eth0@if1120      UP             
eth0.10@eth0     UP             10.10.10.1/24 
eth0.20@eth0     UP             10.10.20.1/24 

CHECK PASS: K4 (EMULATED router) ip_forward=1 

##### Korak 5 – SW2: samo OpenFlow pravila
[sw2]$ ovs-vsctl port-to-br eth0
br0
(rc=0)
[sw2]$ ovs-ofctl del-flows br0
(rc=0)
[VPC1> ping 10.10.20.12 -c 1
host (10.10.10.1) not reachable
(rc=0)
[sw2]$ ovs-ofctl add-flow br0 "priority=100,in_port=eth0,dl_vlan=10,actions=output:eth1"
(rc=0)
[sw2]$ ovs-ofctl add-flow br0 "priority=100,in_port=eth0,dl_vlan=20,actions=output:eth1"
(rc=0)
[sw2]$ ovs-ofctl add-flow br0 "priority=100,in_port=eth1,dl_vlan=10,actions=output:eth0"
(rc=0)
[sw2]$ ovs-ofctl add-flow br0 "priority=100,in_port=eth1,dl_vlan=20,actions=output:eth0"
(rc=0)
[sw2]$ ovs-ofctl add-flow br0 "priority=0,actions=drop"
(rc=0)
[sw2]$ ovs-ofctl --names --no-stats dump-flows br0
 priority=100,in_port=eth0,dl_vlan=10 actions=output:eth1
 priority=100,in_port=eth0,dl_vlan=20 actions=output:eth1
 priority=100,in_port=eth1,dl_vlan=10 actions=output:eth0
 priority=100,in_port=eth1,dl_vlan=20 actions=output:eth0
 priority=0 actions=drop
(rc=0)
CHECK PASS: K5 eth0 in br0 
CHECK PASS: K5 without SW2 flows (no NORMAL): VPC1->VPC2 fails, gateway not reachable host (10.10.10.1) not reachable
CHECK PASS: K5 exactly 5 flows, no NORMAL 

##### Korak 6 – provera
[sw2]$ ovs-ofctl --names dump-flows br0
 cookie=0x0, duration=0.303s, table=0, n_packets=0, n_bytes=0, priority=100,in_port=eth0,dl_vlan=10 actions=output:eth1
 cookie=0x0, duration=0.243s, table=0, n_packets=0, n_bytes=0, priority=100,in_port=eth0,dl_vlan=20 actions=output:eth1
 cookie=0x0, duration=0.194s, table=0, n_packets=0, n_bytes=0, priority=100,in_port=eth1,dl_vlan=10 actions=output:eth0
 cookie=0x0, duration=0.147s, table=0, n_packets=0, n_bytes=0, priority=100,in_port=eth1,dl_vlan=20 actions=output:eth0
 cookie=0x0, duration=0.101s, table=0, n_packets=0, n_bytes=0, priority=0 actions=drop
(rc=0)
[VPC1> ping 10.10.10.1 -c 2
84 bytes from 10.10.10.1 icmp_seq=1 ttl=64 time=1.121 ms
84 bytes from 10.10.10.1 icmp_seq=2 ttl=64 time=1.522 ms
(rc=0)
[VPC1> ping 10.10.20.12 -c 2
84 bytes from 10.10.20.12 icmp_seq=1 ttl=63 time=5.339 ms
84 bytes from 10.10.20.12 icmp_seq=2 ttl=63 time=3.073 ms
(rc=0)
[VPC2> ping 10.10.20.1 -c 2
84 bytes from 10.10.20.1 icmp_seq=1 ttl=64 time=1.673 ms
84 bytes from 10.10.20.1 icmp_seq=2 ttl=64 time=1.463 ms
(rc=0)
[VPC2> ping 10.10.10.11 -c 2
84 bytes from 10.10.10.11 icmp_seq=1 ttl=63 time=2.701 ms
84 bytes from 10.10.10.11 icmp_seq=2 ttl=63 time=2.193 ms
(rc=0)
----- capture K6 sw2eth0 (30 frames)
  13:07:55.519250 00:50:79:66:68:1c > ff:ff:ff:ff:ff:ff, ethertype 802.1Q (0x8100), length 68: vlan 10, p 0, ethertype ARP (0x0806), Ethernet (len 6), IPv4 (len 4), Request who-has 10.10.10.1 (ff:ff:ff:ff:ff:ff) tell 10.10.10.11, length 50
  13:07:55.519797 86:87:3c:21:3b:17 > 00:50:79:66:68:1c, ethertype 802.1Q (0x8100), length 46: vlan 10, p 0, ethertype ARP (0x0806), Ethernet (len 6), IPv4 (len 4), Reply 10.10.10.1 is-at 86:87:3c:21:3b:17, length 28
  13:07:55.521849 00:50:79:66:68:1c > 86:87:3c:21:3b:17, ethertype 802.1Q (0x8100), length 102: vlan 10, p 0, ethertype IPv4 (0x0800), (tos 0x0, ttl 64, id 41771, offset 0, flags [none], proto ICMP (1), length 84) | 10.10.10.11 > 10.10.10.1: ICMP echo request, id 11171, seq 1, length 64
  13:07:55.522298 86:87:3c:21:3b:17 > 00:50:79:66:68:1c, ethertype 802.1Q (0x8100), length 102: vlan 10, p 0, ethertype IPv4 (0x0800), (tos 0x0, ttl 64, id 44115, offset 0, flags [none], proto ICMP (1), length 84) | 10.10.10.1 > 10.10.10.11: ICMP echo reply, id 11171, seq 1, length 64
  13:07:56.523909 00:50:79:66:68:1c > 86:87:3c:21:3b:17, ethertype 802.1Q (0x8100), length 102: vlan 10, p 0, ethertype IPv4 (0x0800), (tos 0x0, ttl 64, id 41772, offset 0, flags [none], proto ICMP (1), length 84) | 10.10.10.11 > 10.10.10.1: ICMP echo request, id 11427, seq 2, length 64
  13:07:56.524453 86:87:3c:21:3b:17 > 00:50:79:66:68:1c, ethertype 802.1Q (0x8100), length 102: vlan 10, p 0, ethertype IPv4 (0x0800), (tos 0x0, ttl 64, id 44501, offset 0, flags [none], proto ICMP (1), length 84) | 10.10.10.1 > 10.10.10.11: ICMP echo reply, id 11427, seq 2, length 64
  13:07:57.546890 00:50:79:66:68:1c > 86:87:3c:21:3b:17, ethertype 802.1Q (0x8100), length 102: vlan 10, p 0, ethertype IPv4 (0x0800), (tos 0x0, ttl 64, id 41773, offset 0, flags [none], proto ICMP (1), length 84) | 10.10.10.11 > 10.10.20.12: ICMP echo request, id 11683, seq 1, length 64
  13:07:57.547432 86:87:3c:21:3b:17 > ff:ff:ff:ff:ff:ff, ethertype 802.1Q (0x8100), length 46: vlan 20, p 0, ethertype ARP (0x0806), Ethernet (len 6), IPv4 (len 4), Request who-has 10.10.20.12 tell 10.10.20.1, length 28
  13:07:57.548323 00:50:79:66:68:a7 > 86:87:3c:21:3b:17, ethertype 802.1Q (0x8100), length 46: vlan 20, p 0, ethertype ARP (0x0806), Ethernet (len 6), IPv4 (len 4), Reply 10.10.20.12 is-at 00:50:79:66:68:a7, length 28
  13:07:57.548790 86:87:3c:21:3b:17 > 00:50:79:66:68:a7, ethertype 802.1Q (0x8100), length 102: vlan 20, p 0, ethertype IPv4 (0x0800), (tos 0x0, ttl 63, id 41773, offset 0, flags [none], proto ICMP (1), length 84) | 10.10.10.11 > 10.10.20.12: ICMP echo request, id 11683, seq 1, length 64
  13:07:57.550656 00:50:79:66:68:a7 > 86:87:3c:21:3b:17, ethertype 802.1Q (0x8100), length 102: vlan 20, p 0, ethertype IPv4 (0x0800), (tos 0x0, ttl 64, id 41773, offset 0, flags [none], proto ICMP (1), length 84) | 10.10.20.12 > 10.10.10.11: ICMP echo reply, id 11683, seq 1, length 64
  13:07:57.551588 86:87:3c:21:3b:17 > 00:50:79:66:68:1c, ethertype 802.1Q (0x8100), length 102: vlan 10, p 0, ethertype IPv4 (0x0800), (tos 0x0, ttl 63, id 41773, offset 0, flags [none], proto ICMP (1), length 84) | 10.10.20.12 > 10.10.10.11: ICMP echo reply, id 11683, seq 1, length 64
  13:07:58.553296 00:50:79:66:68:1c > 86:87:3c:21:3b:17, ethertype 802.1Q (0x8100), length 102: vlan 10, p 0, ethertype IPv4 (0x0800), (tos 0x0, ttl 64, id 41774, offset 0, flags [none], proto ICMP (1), length 84) | 10.10.10.11 > 10.10.20.12: ICMP echo request, id 11939, seq 2, length 64
  13:07:58.553828 86:87:3c:21:3b:17 > 00:50:79:66:68:a7, ethertype 802.1Q (0x8100), length 102: vlan 20, p 0, ethertype IPv4 (0x0800), (tos 0x0, ttl 63, id 41774, offset 0, flags [none], proto ICMP (1), length 84) | 10.10.10.11 > 10.10.20.12: ICMP echo request, id 11939, seq 2, length 64
  13:07:58.555043 00:50:79:66:68:a7 > 86:87:3c:21:3b:17, ethertype 802.1Q (0x8100), length 102: vlan 20, p 0, ethertype IPv4 (0x0800), (tos 0x0, ttl 64, id 41774, offset 0, flags [none], proto ICMP (1), length 84) | 10.10.20.12 > 10.10.10.11: ICMP echo reply, id 11939, seq 2, length 64
  13:07:58.555476 86:87:3c:21:3b:17 > 00:50:79:66:68:1c, ethertype 802.1Q (0x8100), length 102: vlan 10, p 0, ethertype IPv4 (0x0800), (tos 0x0, ttl 63, id 41774, offset 0, flags [none], proto ICMP (1), length 84) | 10.10.20.12 > 10.10.10.11: ICMP echo reply, id 11939, seq 2, length 64
  13:07:59.563725 00:50:79:66:68:a7 > 86:87:3c:21:3b:17, ethertype 802.1Q (0x8100), length 102: vlan 20, p 0, ethertype IPv4 (0x0800), (tos 0x0, ttl 64, id 41775, offset 0, flags [none], proto ICMP (1), length 84) | 10.10.20.12 > 10.10.20.1: ICMP echo request, id 12195, seq 1, length 64
  13:07:59.564252 86:87:3c:21:3b:17 > 00:50:79:66:68:a7, ethertype 802.1Q (0x8100), length 102: vlan 20, p 0, ethertype IPv4 (0x0800), (tos 0x0, ttl 64, id 25886, offset 0, flags [none], proto ICMP (1), length 84) | 10.10.20.1 > 10.10.20.12: ICMP echo reply, id 12195, seq 1, length 64
  13:08:00.566692 00:50:79:66:68:a7 > 86:87:3c:21:3b:17, ethertype 802.1Q (0x8100), length 102: vlan 20, p 0, ethertype IPv4 (0x0800), (tos 0x0, ttl 64, id 41776, offset 0, flags [none], proto ICMP (1), length 84) | 10.10.20.12 > 10.10.20.1: ICMP echo request, id 12451, seq 2, length 64
  13:08:00.567301 86:87:3c:21:3b:17 > 00:50:79:66:68:a7, ethertype 802.1Q (0x8100), length 102: vlan 20, p 0, ethertype IPv4 (0x0800), (tos 0x0, ttl 64, id 26522, offset 0, flags [none], proto ICMP (1), length 84) | 10.10.20.1 > 10.10.20.12: ICMP echo reply, id 12451, seq 2, length 64
  13:08:01.005432 86:87:3c:21:3b:17 > 00:50:79:66:68:1c, ethertype 802.1Q (0x8100), length 46: vlan 10, p 0, ethertype ARP (0x0806), Ethernet (len 6), IPv4 (len 4), Request who-has 10.10.10.11 tell 10.10.10.1, length 28
  13:08:01.006922 00:50:79:66:68:1c > 86:87:3c:21:3b:17, ethertype 802.1Q (0x8100), length 46: vlan 10, p 0, ethertype ARP (0x0806), Ethernet (len 6), IPv4 (len 4), Reply 10.10.10.11 is-at 00:50:79:66:68:1c, length 28
  13:08:01.583804 00:50:79:66:68:a7 > 86:87:3c:21:3b:17, ethertype 802.1Q (0x8100), length 102: vlan 20, p 0, ethertype IPv4 (0x0800), (tos 0x0, ttl 64, id 41777, offset 0, flags [none], proto ICMP (1), length 84) | 10.10.20.12 > 10.10.10.11: ICMP echo request, id 12707, seq 1, length 64
  13:08:01.584301 86:87:3c:21:3b:17 > 00:50:79:66:68:1c, ethertype 802.1Q (0x8100), length 102: vlan 10, p 0, ethertype IPv4 (0x0800), (tos 0x0, ttl 63, id 41777, offset 0, flags [none], proto ICMP (1), length 84) | 10.10.20.12 > 10.10.10.11: ICMP echo request, id 12707, seq 1, length 64
  13:08:01.585159 00:50:79:66:68:1c > 86:87:3c:21:3b:17, ethertype 802.1Q (0x8100), length 102: vlan 10, p 0, ethertype IPv4 (0x0800), (tos 0x0, ttl 64, id 41777, offset 0, flags [none], proto ICMP (1), length 84) | 10.10.10.11 > 10.10.20.12: ICMP echo reply, id 12707, seq 1, length 64
  13:08:01.585587 86:87:3c:21:3b:17 > 00:50:79:66:68:a7, ethertype 802.1Q (0x8100), length 102: vlan 20, p 0, ethertype IPv4 (0x0800), (tos 0x0, ttl 63, id 41777, offset 0, flags [none], proto ICMP (1), length 84) | 10.10.10.11 > 10.10.20.12: ICMP echo reply, id 12707, seq 1, length 64
  13:08:02.586852 00:50:79:66:68:a7 > 86:87:3c:21:3b:17, ethertype 802.1Q (0x8100), length 102: vlan 20, p 0, ethertype IPv4 (0x0800), (tos 0x0, ttl 64, id 41778, offset 0, flags [none], proto ICMP (1), length 84) | 10.10.20.12 > 10.10.10.11: ICMP echo request, id 12963, seq 2, length 64
  13:08:02.587355 86:87:3c:21:3b:17 > 00:50:79:66:68:1c, ethertype 802.1Q (0x8100), length 102: vlan 10, p 0, ethertype IPv4 (0x0800), (tos 0x0, ttl 63, id 41778, offset 0, flags [none], proto ICMP (1), length 84) | 10.10.20.12 > 10.10.10.11: ICMP echo request, id 12963, seq 2, length 64
  13:08:02.587896 00:50:79:66:68:1c > 86:87:3c:21:3b:17, ethertype 802.1Q (0x8100), length 102: vlan 10, p 0, ethertype IPv4 (0x0800), (tos 0x0, ttl 64, id 41778, offset 0, flags [none], proto ICMP (1), length 84) | 10.10.10.11 > 10.10.20.12: ICMP echo reply, id 12963, seq 2, length 64
  13:08:02.588306 86:87:3c:21:3b:17 > 00:50:79:66:68:a7, ethertype 802.1Q (0x8100), length 102: vlan 20, p 0, ethertype IPv4 (0x0800), (tos 0x0, ttl 63, id 41778, offset 0, flags [none], proto ICMP (1), length 84) | 10.10.10.11 > 10.10.20.12: ICMP echo reply, id 12963, seq 2, length 64
----- capture K6 sw2eth1 (32 frames)
  13:07:55.519558 00:50:79:66:68:1c > ff:ff:ff:ff:ff:ff, ethertype 802.1Q (0x8100), length 68: vlan 10, p 0, ethertype ARP (0x0806), Ethernet (len 6), IPv4 (len 4), Request who-has 10.10.10.1 (ff:ff:ff:ff:ff:ff) tell 10.10.10.11, length 50
  13:07:55.519589 86:87:3c:21:3b:17 > 00:50:79:66:68:1c, ethertype 802.1Q (0x8100), length 46: vlan 10, p 0, ethertype ARP (0x0806), Ethernet (len 6), IPv4 (len 4), Reply 10.10.10.1 is-at 86:87:3c:21:3b:17, length 28
  13:07:55.522093 00:50:79:66:68:1c > 86:87:3c:21:3b:17, ethertype 802.1Q (0x8100), length 102: vlan 10, p 0, ethertype IPv4 (0x0800), (tos 0x0, ttl 64, id 41771, offset 0, flags [none], proto ICMP (1), length 84) | 10.10.10.11 > 10.10.10.1: ICMP echo request, id 11171, seq 1, length 64
  13:07:55.522126 86:87:3c:21:3b:17 > 00:50:79:66:68:1c, ethertype 802.1Q (0x8100), length 102: vlan 10, p 0, ethertype IPv4 (0x0800), (tos 0x0, ttl 64, id 44115, offset 0, flags [none], proto ICMP (1), length 84) | 10.10.10.1 > 10.10.10.11: ICMP echo reply, id 11171, seq 1, length 64
  13:07:56.524231 00:50:79:66:68:1c > 86:87:3c:21:3b:17, ethertype 802.1Q (0x8100), length 102: vlan 10, p 0, ethertype IPv4 (0x0800), (tos 0x0, ttl 64, id 41772, offset 0, flags [none], proto ICMP (1), length 84) | 10.10.10.11 > 10.10.10.1: ICMP echo request, id 11427, seq 2, length 64
  13:07:56.524276 86:87:3c:21:3b:17 > 00:50:79:66:68:1c, ethertype 802.1Q (0x8100), length 102: vlan 10, p 0, ethertype IPv4 (0x0800), (tos 0x0, ttl 64, id 44501, offset 0, flags [none], proto ICMP (1), length 84) | 10.10.10.1 > 10.10.10.11: ICMP echo reply, id 11427, seq 2, length 64
  13:07:57.547188 00:50:79:66:68:1c > 86:87:3c:21:3b:17, ethertype 802.1Q (0x8100), length 102: vlan 10, p 0, ethertype IPv4 (0x0800), (tos 0x0, ttl 64, id 41773, offset 0, flags [none], proto ICMP (1), length 84) | 10.10.10.11 > 10.10.20.12: ICMP echo request, id 11683, seq 1, length 64
  13:07:57.547231 86:87:3c:21:3b:17 > ff:ff:ff:ff:ff:ff, ethertype 802.1Q (0x8100), length 46: vlan 20, p 0, ethertype ARP (0x0806), Ethernet (len 6), IPv4 (len 4), Request who-has 10.10.20.12 tell 10.10.20.1, length 28
  13:07:57.548559 00:50:79:66:68:a7 > 86:87:3c:21:3b:17, ethertype 802.1Q (0x8100), length 46: vlan 20, p 0, ethertype ARP (0x0806), Ethernet (len 6), IPv4 (len 4), Reply 10.10.20.12 is-at 00:50:79:66:68:a7, length 28
  13:07:57.548575 86:87:3c:21:3b:17 > 00:50:79:66:68:a7, ethertype 802.1Q (0x8100), length 102: vlan 20, p 0, ethertype IPv4 (0x0800), (tos 0x0, ttl 63, id 41773, offset 0, flags [none], proto ICMP (1), length 84) | 10.10.10.11 > 10.10.20.12: ICMP echo request, id 11683, seq 1, length 64
  13:07:57.551118 00:50:79:66:68:a7 > 86:87:3c:21:3b:17, ethertype 802.1Q (0x8100), length 102: vlan 20, p 0, ethertype IPv4 (0x0800), (tos 0x0, ttl 64, id 41773, offset 0, flags [none], proto ICMP (1), length 84) | 10.10.20.12 > 10.10.10.11: ICMP echo reply, id 11683, seq 1, length 64
  13:07:57.551144 86:87:3c:21:3b:17 > 00:50:79:66:68:1c, ethertype 802.1Q (0x8100), length 102: vlan 10, p 0, ethertype IPv4 (0x0800), (tos 0x0, ttl 63, id 41773, offset 0, flags [none], proto ICMP (1), length 84) | 10.10.20.12 > 10.10.10.11: ICMP echo reply, id 11683, seq 1, length 64
  13:07:58.553610 00:50:79:66:68:1c > 86:87:3c:21:3b:17, ethertype 802.1Q (0x8100), length 102: vlan 10, p 0, ethertype IPv4 (0x0800), (tos 0x0, ttl 64, id 41774, offset 0, flags [none], proto ICMP (1), length 84) | 10.10.10.11 > 10.10.20.12: ICMP echo request, id 11939, seq 2, length 64
  13:07:58.553645 86:87:3c:21:3b:17 > 00:50:79:66:68:a7, ethertype 802.1Q (0x8100), length 102: vlan 20, p 0, ethertype IPv4 (0x0800), (tos 0x0, ttl 63, id 41774, offset 0, flags [none], proto ICMP (1), length 84) | 10.10.10.11 > 10.10.20.12: ICMP echo request, id 11939, seq 2, length 64
  13:07:58.555290 00:50:79:66:68:a7 > 86:87:3c:21:3b:17, ethertype 802.1Q (0x8100), length 102: vlan 20, p 0, ethertype IPv4 (0x0800), (tos 0x0, ttl 64, id 41774, offset 0, flags [none], proto ICMP (1), length 84) | 10.10.20.12 > 10.10.10.11: ICMP echo reply, id 11939, seq 2, length 64
  13:07:58.555304 86:87:3c:21:3b:17 > 00:50:79:66:68:1c, ethertype 802.1Q (0x8100), length 102: vlan 10, p 0, ethertype IPv4 (0x0800), (tos 0x0, ttl 63, id 41774, offset 0, flags [none], proto ICMP (1), length 84) | 10.10.20.12 > 10.10.10.11: ICMP echo reply, id 11939, seq 2, length 64
  13:07:59.564021 00:50:79:66:68:a7 > 86:87:3c:21:3b:17, ethertype 802.1Q (0x8100), length 102: vlan 20, p 0, ethertype IPv4 (0x0800), (tos 0x0, ttl 64, id 41775, offset 0, flags [none], proto ICMP (1), length 84) | 10.10.20.12 > 10.10.20.1: ICMP echo request, id 12195, seq 1, length 64
  13:07:59.564061 86:87:3c:21:3b:17 > 00:50:79:66:68:a7, ethertype 802.1Q (0x8100), length 102: vlan 20, p 0, ethertype IPv4 (0x0800), (tos 0x0, ttl 64, id 25886, offset 0, flags [none], proto ICMP (1), length 84) | 10.10.20.1 > 10.10.20.12: ICMP echo reply, id 12195, seq 1, length 64
  13:08:00.567052 00:50:79:66:68:a7 > 86:87:3c:21:3b:17, ethertype 802.1Q (0x8100), length 102: vlan 20, p 0, ethertype IPv4 (0x0800), (tos 0x0, ttl 64, id 41776, offset 0, flags [none], proto ICMP (1), length 84) | 10.10.20.12 > 10.10.20.1: ICMP echo request, id 12451, seq 2, length 64
  13:08:00.567092 86:87:3c:21:3b:17 > 00:50:79:66:68:a7, ethertype 802.1Q (0x8100), length 102: vlan 20, p 0, ethertype IPv4 (0x0800), (tos 0x0, ttl 64, id 26522, offset 0, flags [none], proto ICMP (1), length 84) | 10.10.20.1 > 10.10.20.12: ICMP echo reply, id 12451, seq 2, length 64
  13:08:01.005198 86:87:3c:21:3b:17 > 00:50:79:66:68:1c, ethertype 802.1Q (0x8100), length 46: vlan 10, p 0, ethertype ARP (0x0806), Ethernet (len 6), IPv4 (len 4), Request who-has 10.10.10.11 tell 10.10.10.1, length 28
  13:08:01.005862 8a:6e:e9:39:58:5e > 33:33:00:00:00:02, ethertype 802.1Q (0x8100), length 74: vlan 10, p 0, ethertype IPv6 (0x86dd), (hlim 255, next-header ICMPv6 (58), payload length 16) fe80::886e:e9ff:fe39:585e > ff02::2: [icmp6 sum ok] ICMP6, router solicitation, length 16 | source link-address option (1), length 8 (1): 8a:6e:e9:39:58:5e
  13:08:01.007315 00:50:79:66:68:1c > 86:87:3c:21:3b:17, ethertype 802.1Q (0x8100), length 46: vlan 10, p 0, ethertype ARP (0x0806), Ethernet (len 6), IPv4 (len 4), Reply 10.10.10.11 is-at 00:50:79:66:68:1c, length 28
  13:08:01.584078 00:50:79:66:68:a7 > 86:87:3c:21:3b:17, ethertype 802.1Q (0x8100), length 102: vlan 20, p 0, ethertype IPv4 (0x0800), (tos 0x0, ttl 64, id 41777, offset 0, flags [none], proto ICMP (1), length 84) | 10.10.20.12 > 10.10.10.11: ICMP echo request, id 12707, seq 1, length 64
  13:08:01.584110 86:87:3c:21:3b:17 > 00:50:79:66:68:1c, ethertype 802.1Q (0x8100), length 102: vlan 10, p 0, ethertype IPv4 (0x0800), (tos 0x0, ttl 63, id 41777, offset 0, flags [none], proto ICMP (1), length 84) | 10.10.20.12 > 10.10.10.11: ICMP echo request, id 12707, seq 1, length 64
  13:08:01.585429 00:50:79:66:68:1c > 86:87:3c:21:3b:17, ethertype 802.1Q (0x8100), length 102: vlan 10, p 0, ethertype IPv4 (0x0800), (tos 0x0, ttl 64, id 41777, offset 0, flags [none], proto ICMP (1), length 84) | 10.10.10.11 > 10.10.20.12: ICMP echo reply, id 12707, seq 1, length 64
  13:08:01.585444 86:87:3c:21:3b:17 > 00:50:79:66:68:a7, ethertype 802.1Q (0x8100), length 102: vlan 20, p 0, ethertype IPv4 (0x0800), (tos 0x0, ttl 63, id 41777, offset 0, flags [none], proto ICMP (1), length 84) | 10.10.10.11 > 10.10.20.12: ICMP echo reply, id 12707, seq 1, length 64
  13:08:02.031524 42:6e:1c:25:e6:dd > 33:33:00:00:00:02, ethertype 802.1Q (0x8100), length 74: vlan 20, p 0, ethertype IPv6 (0x86dd), (hlim 255, next-header ICMPv6 (58), payload length 16) fe80::6089:17ff:fef5:70b8 > ff02::2: [icmp6 sum ok] ICMP6, router solicitation, length 16 | source link-address option (1), length 8 (1): 42:6e:1c:25:e6:dd
  13:08:02.587176 00:50:79:66:68:a7 > 86:87:3c:21:3b:17, ethertype 802.1Q (0x8100), length 102: vlan 20, p 0, ethertype IPv4 (0x0800), (tos 0x0, ttl 64, id 41778, offset 0, flags [none], proto ICMP (1), length 84) | 10.10.20.12 > 10.10.10.11: ICMP echo request, id 12963, seq 2, length 64
  13:08:02.587208 86:87:3c:21:3b:17 > 00:50:79:66:68:1c, ethertype 802.1Q (0x8100), length 102: vlan 10, p 0, ethertype IPv4 (0x0800), (tos 0x0, ttl 63, id 41778, offset 0, flags [none], proto ICMP (1), length 84) | 10.10.20.12 > 10.10.10.11: ICMP echo request, id 12963, seq 2, length 64
  13:08:02.588140 00:50:79:66:68:1c > 86:87:3c:21:3b:17, ethertype 802.1Q (0x8100), length 102: vlan 10, p 0, ethertype IPv4 (0x0800), (tos 0x0, ttl 64, id 41778, offset 0, flags [none], proto ICMP (1), length 84) | 10.10.10.11 > 10.10.20.12: ICMP echo reply, id 12963, seq 2, length 64
  13:08:02.588156 86:87:3c:21:3b:17 > 00:50:79:66:68:a7, ethertype 802.1Q (0x8100), length 102: vlan 20, p 0, ethertype IPv4 (0x0800), (tos 0x0, ttl 63, id 41778, offset 0, flags [none], proto ICMP (1), length 84) | 10.10.10.11 > 10.10.20.12: ICMP echo reply, id 12963, seq 2, length 64
----- capture K6 vpc1 (16 frames)
  13:07:55.518949 00:50:79:66:68:1c > ff:ff:ff:ff:ff:ff, ethertype ARP (0x0806), length 64: Ethernet (len 6), IPv4 (len 4), Request who-has 10.10.10.1 (ff:ff:ff:ff:ff:ff) tell 10.10.10.11, length 50
  13:07:55.520099 86:87:3c:21:3b:17 > 00:50:79:66:68:1c, ethertype ARP (0x0806), length 42: Ethernet (len 6), IPv4 (len 4), Reply 10.10.10.1 is-at 86:87:3c:21:3b:17, length 28
  13:07:55.521645 00:50:79:66:68:1c > 86:87:3c:21:3b:17, ethertype IPv4 (0x0800), length 98: (tos 0x0, ttl 64, id 41771, offset 0, flags [none], proto ICMP (1), length 84) | 10.10.10.11 > 10.10.10.1: ICMP echo request, id 11171, seq 1, length 64
  13:07:55.522600 86:87:3c:21:3b:17 > 00:50:79:66:68:1c, ethertype IPv4 (0x0800), length 98: (tos 0x0, ttl 64, id 44115, offset 0, flags [none], proto ICMP (1), length 84) | 10.10.10.1 > 10.10.10.11: ICMP echo reply, id 11171, seq 1, length 64
  13:07:56.523661 00:50:79:66:68:1c > 86:87:3c:21:3b:17, ethertype IPv4 (0x0800), length 98: (tos 0x0, ttl 64, id 41772, offset 0, flags [none], proto ICMP (1), length 84) | 10.10.10.11 > 10.10.10.1: ICMP echo request, id 11427, seq 2, length 64
  13:07:56.524766 86:87:3c:21:3b:17 > 00:50:79:66:68:1c, ethertype IPv4 (0x0800), length 98: (tos 0x0, ttl 64, id 44501, offset 0, flags [none], proto ICMP (1), length 84) | 10.10.10.1 > 10.10.10.11: ICMP echo reply, id 11427, seq 2, length 64
  13:07:57.546643 00:50:79:66:68:1c > 86:87:3c:21:3b:17, ethertype IPv4 (0x0800), length 98: (tos 0x0, ttl 64, id 41773, offset 0, flags [none], proto ICMP (1), length 84) | 10.10.10.11 > 10.10.20.12: ICMP echo request, id 11683, seq 1, length 64
  13:07:57.551868 86:87:3c:21:3b:17 > 00:50:79:66:68:1c, ethertype IPv4 (0x0800), length 98: (tos 0x0, ttl 63, id 41773, offset 0, flags [none], proto ICMP (1), length 84) | 10.10.20.12 > 10.10.10.11: ICMP echo reply, id 11683, seq 1, length 64
  13:07:58.552982 00:50:79:66:68:1c > 86:87:3c:21:3b:17, ethertype IPv4 (0x0800), length 98: (tos 0x0, ttl 64, id 41774, offset 0, flags [none], proto ICMP (1), length 84) | 10.10.10.11 > 10.10.20.12: ICMP echo request, id 11939, seq 2, length 64
  13:07:58.555732 86:87:3c:21:3b:17 > 00:50:79:66:68:1c, ethertype IPv4 (0x0800), length 98: (tos 0x0, ttl 63, id 41774, offset 0, flags [none], proto ICMP (1), length 84) | 10.10.20.12 > 10.10.10.11: ICMP echo reply, id 11939, seq 2, length 64
  13:08:01.005649 86:87:3c:21:3b:17 > 00:50:79:66:68:1c, ethertype ARP (0x0806), length 42: Ethernet (len 6), IPv4 (len 4), Request who-has 10.10.10.11 tell 10.10.10.1, length 28
  13:08:01.006075 00:50:79:66:68:1c > 86:87:3c:21:3b:17, ethertype ARP (0x0806), length 42: Ethernet (len 6), IPv4 (len 4), Reply 10.10.10.11 is-at 00:50:79:66:68:1c, length 28
  13:08:01.584685 86:87:3c:21:3b:17 > 00:50:79:66:68:1c, ethertype IPv4 (0x0800), length 98: (tos 0x0, ttl 63, id 41777, offset 0, flags [none], proto ICMP (1), length 84) | 10.10.20.12 > 10.10.10.11: ICMP echo request, id 12707, seq 1, length 64
  13:08:01.584999 00:50:79:66:68:1c > 86:87:3c:21:3b:17, ethertype IPv4 (0x0800), length 98: (tos 0x0, ttl 64, id 41777, offset 0, flags [none], proto ICMP (1), length 84) | 10.10.10.11 > 10.10.20.12: ICMP echo reply, id 12707, seq 1, length 64
  13:08:02.587548 86:87:3c:21:3b:17 > 00:50:79:66:68:1c, ethertype IPv4 (0x0800), length 98: (tos 0x0, ttl 63, id 41778, offset 0, flags [none], proto ICMP (1), length 84) | 10.10.20.12 > 10.10.10.11: ICMP echo request, id 12963, seq 2, length 64
  13:08:02.587735 00:50:79:66:68:1c > 86:87:3c:21:3b:17, ethertype IPv4 (0x0800), length 98: (tos 0x0, ttl 64, id 41778, offset 0, flags [none], proto ICMP (1), length 84) | 10.10.10.11 > 10.10.20.12: ICMP echo reply, id 12963, seq 2, length 64
----- capture K6 vpc2 (14 frames)
  13:07:57.547782 86:87:3c:21:3b:17 > ff:ff:ff:ff:ff:ff, ethertype ARP (0x0806), length 42: Ethernet (len 6), IPv4 (len 4), Request who-has 10.10.20.12 tell 10.10.20.1, length 28
  13:07:57.547933 00:50:79:66:68:a7 > 86:87:3c:21:3b:17, ethertype ARP (0x0806), length 42: Ethernet (len 6), IPv4 (len 4), Reply 10.10.20.12 is-at 00:50:79:66:68:a7, length 28
  13:07:57.550069 86:87:3c:21:3b:17 > 00:50:79:66:68:a7, ethertype IPv4 (0x0800), length 98: (tos 0x0, ttl 63, id 41773, offset 0, flags [none], proto ICMP (1), length 84) | 10.10.10.11 > 10.10.20.12: ICMP echo request, id 11683, seq 1, length 64
  13:07:57.550216 00:50:79:66:68:a7 > 86:87:3c:21:3b:17, ethertype IPv4 (0x0800), length 98: (tos 0x0, ttl 64, id 41773, offset 0, flags [none], proto ICMP (1), length 84) | 10.10.20.12 > 10.10.10.11: ICMP echo reply, id 11683, seq 1, length 64
  13:07:58.554190 86:87:3c:21:3b:17 > 00:50:79:66:68:a7, ethertype IPv4 (0x0800), length 98: (tos 0x0, ttl 63, id 41774, offset 0, flags [none], proto ICMP (1), length 84) | 10.10.10.11 > 10.10.20.12: ICMP echo request, id 11939, seq 2, length 64
  13:07:58.554672 00:50:79:66:68:a7 > 86:87:3c:21:3b:17, ethertype IPv4 (0x0800), length 98: (tos 0x0, ttl 64, id 41774, offset 0, flags [none], proto ICMP (1), length 84) | 10.10.20.12 > 10.10.10.11: ICMP echo reply, id 11939, seq 2, length 64
  13:07:59.563300 00:50:79:66:68:a7 > 86:87:3c:21:3b:17, ethertype IPv4 (0x0800), length 98: (tos 0x0, ttl 64, id 41775, offset 0, flags [none], proto ICMP (1), length 84) | 10.10.20.12 > 10.10.20.1: ICMP echo request, id 12195, seq 1, length 64
  13:07:59.564673 86:87:3c:21:3b:17 > 00:50:79:66:68:a7, ethertype IPv4 (0x0800), length 98: (tos 0x0, ttl 64, id 25886, offset 0, flags [none], proto ICMP (1), length 84) | 10.10.20.1 > 10.10.20.12: ICMP echo reply, id 12195, seq 1, length 64
  13:08:00.566253 00:50:79:66:68:a7 > 86:87:3c:21:3b:17, ethertype IPv4 (0x0800), length 98: (tos 0x0, ttl 64, id 41776, offset 0, flags [none], proto ICMP (1), length 84) | 10.10.20.12 > 10.10.20.1: ICMP echo request, id 12451, seq 2, length 64
  13:08:00.567557 86:87:3c:21:3b:17 > 00:50:79:66:68:a7, ethertype IPv4 (0x0800), length 98: (tos 0x0, ttl 64, id 26522, offset 0, flags [none], proto ICMP (1), length 84) | 10.10.20.1 > 10.10.20.12: ICMP echo reply, id 12451, seq 2, length 64
  13:08:01.583424 00:50:79:66:68:a7 > 86:87:3c:21:3b:17, ethertype IPv4 (0x0800), length 98: (tos 0x0, ttl 64, id 41777, offset 0, flags [none], proto ICMP (1), length 84) | 10.10.20.12 > 10.10.10.11: ICMP echo request, id 12707, seq 1, length 64
  13:08:01.585855 86:87:3c:21:3b:17 > 00:50:79:66:68:a7, ethertype IPv4 (0x0800), length 98: (tos 0x0, ttl 63, id 41777, offset 0, flags [none], proto ICMP (1), length 84) | 10.10.10.11 > 10.10.20.12: ICMP echo reply, id 12707, seq 1, length 64
  13:08:02.586503 00:50:79:66:68:a7 > 86:87:3c:21:3b:17, ethertype IPv4 (0x0800), length 98: (tos 0x0, ttl 64, id 41778, offset 0, flags [none], proto ICMP (1), length 84) | 10.10.20.12 > 10.10.10.11: ICMP echo request, id 12963, seq 2, length 64
  13:08:02.588563 86:87:3c:21:3b:17 > 00:50:79:66:68:a7, ethertype IPv4 (0x0800), length 98: (tos 0x0, ttl 63, id 41778, offset 0, flags [none], proto ICMP (1), length 84) | 10.10.10.11 > 10.10.20.12: ICMP echo reply, id 12963, seq 2, length 64
CHECK PASS: K6 VPC1 -> gateway 10.10.10.1 84 bytes from 10.10.10.1 icmp_seq=1 ttl=64 time=1.121 ms
84 bytes from 10.10.10.1 icmp_seq=2 ttl=64 time=1.522 ms
CHECK PASS: K6 VPC1 -> VPC2 ttl=63 84 bytes from 10.10.20.12 icmp_seq=1 ttl=63 time=5.339 ms
84 bytes from 10.10.20.12 icmp_seq=2 ttl=63 time=3.073 ms
CHECK PASS: K6 VPC2 -> gateway 10.10.20.1 84 bytes from 10.10.20.1 icmp_seq=1 ttl=64 time=1.673 ms
84 bytes from 10.10.20.1 icmp_seq=2 ttl=64 time=1.463 ms
CHECK PASS: K6 VPC2 -> VPC1 ttl=63 84 bytes from 10.10.10.11 icmp_seq=1 ttl=63 time=2.701 ms
84 bytes from 10.10.10.11 icmp_seq=2 ttl=63 time=2.193 ms
CHECK PASS: K6 sw2eth0: VPC1->VPC2 request vlan 10 ttl 64 00:50:79:66:68:1c > router 
CHECK PASS: K6 sw2eth0: VPC1->VPC2 request vlan 20 ttl 63 router > 00:50:79:66:68:a7 
CHECK PASS: K6 sw2eth0: every frame is 802.1Q tagged (10 or 20) 
CHECK PASS: K6 sw2eth1: VPC1->VPC2 request vlan 10 ttl 64 00:50:79:66:68:1c > router 
CHECK PASS: K6 sw2eth1: VPC1->VPC2 request vlan 20 ttl 63 router > 00:50:79:66:68:a7 
CHECK PASS: K6 sw2eth1: every frame is 802.1Q tagged (10 or 20) 
CHECK PASS: K6 vpc1 cable: no 802.1Q frames 
CHECK PASS: K6 vpc2 cable: no 802.1Q frames 
CHECK PASS: K6 VPC2 cable: request from router MAC ttl 63 
[sw2]$ ovs-ofctl --names dump-flows br0
 cookie=0x0, duration=12.007s, table=0, n_packets=9, n_bytes=800, priority=100,in_port=eth0,dl_vlan=10 actions=output:eth1
 cookie=0x0, duration=11.947s, table=0, n_packets=8, n_bytes=732, priority=100,in_port=eth0,dl_vlan=20 actions=output:eth1
 cookie=0x0, duration=11.898s, table=0, n_packets=8, n_bytes=704, priority=100,in_port=eth1,dl_vlan=10 actions=output:eth0
 cookie=0x0, duration=11.851s, table=0, n_packets=7, n_bytes=658, priority=100,in_port=eth1,dl_vlan=20 actions=output:eth0
 cookie=0x0, duration=11.805s, table=0, n_packets=3, n_bytes=210, priority=0 actions=drop
(rc=0)
flow deltas K6 {'priority=100,in_port=eth0,dl_vlan=10': 9, 'priority=100,in_port=eth0,dl_vlan=20': 8, 'priority=100,in_port=eth1,dl_vlan=10': 8, 'priority=100,in_port=eth1,dl_vlan=20': 7, 'priority=0': 3}
CHECK PASS: K6 all four forwarding flows used 

##### NEG: VLAN 30 kroz SW2 (harness-only)
[sw2]$ ovs-ofctl --names dump-flows br0
 cookie=0x0, duration=12.283s, table=0, n_packets=9, n_bytes=800, priority=100,in_port=eth0,dl_vlan=10 actions=output:eth1
 cookie=0x0, duration=12.223s, table=0, n_packets=8, n_bytes=732, priority=100,in_port=eth0,dl_vlan=20 actions=output:eth1
 cookie=0x0, duration=12.174s, table=0, n_packets=8, n_bytes=704, priority=100,in_port=eth1,dl_vlan=10 actions=output:eth0
 cookie=0x0, duration=12.127s, table=0, n_packets=7, n_bytes=658, priority=100,in_port=eth1,dl_vlan=20 actions=output:eth0
 cookie=0x0, duration=12.081s, table=0, n_packets=4, n_bytes=304, priority=0 actions=drop
(rc=0)
PING 10.10.30.1 (10.10.30.1): 56 data bytes

--- 10.10.30.1 ping statistics ---
3 packets transmitted, 0 packets received, 100% packet loss
 PING 10.10.30.2 (10.10.30.2): 56 data bytes

--- 10.10.30.2 ping statistics ---
3 packets transmitted, 0 packets received, 100% packet loss

----- capture NEG30 sw2eth0 (3 frames)
  13:08:07.417862 ba:cb:76:42:64:28 > ff:ff:ff:ff:ff:ff, ethertype 802.1Q (0x8100), length 46: vlan 30, p 0, ethertype ARP (0x0806), Ethernet (len 6), IPv4 (len 4), Request who-has 10.10.30.1 tell 10.10.30.2, length 28
  13:08:08.429660 ba:cb:76:42:64:28 > ff:ff:ff:ff:ff:ff, ethertype 802.1Q (0x8100), length 46: vlan 30, p 0, ethertype ARP (0x0806), Ethernet (len 6), IPv4 (len 4), Request who-has 10.10.30.1 tell 10.10.30.2, length 28
  13:08:09.453186 ba:cb:76:42:64:28 > ff:ff:ff:ff:ff:ff, ethertype 802.1Q (0x8100), length 46: vlan 30, p 0, ethertype ARP (0x0806), Ethernet (len 6), IPv4 (len 4), Request who-has 10.10.30.1 tell 10.10.30.2, length 28
----- capture NEG30 sw2eth1 (3 frames)
  13:08:10.462562 86:87:3c:21:3b:17 > ff:ff:ff:ff:ff:ff, ethertype 802.1Q (0x8100), length 46: vlan 30, p 0, ethertype ARP (0x0806), Ethernet (len 6), IPv4 (len 4), Request who-has 10.10.30.2 tell 10.10.30.1, length 28
  13:08:11.501191 86:87:3c:21:3b:17 > ff:ff:ff:ff:ff:ff, ethertype 802.1Q (0x8100), length 46: vlan 30, p 0, ethertype ARP (0x0806), Ethernet (len 6), IPv4 (len 4), Request who-has 10.10.30.2 tell 10.10.30.1, length 28
  13:08:12.525228 86:87:3c:21:3b:17 > ff:ff:ff:ff:ff:ff, ethertype 802.1Q (0x8100), length 46: vlan 30, p 0, ethertype ARP (0x0806), Ethernet (len 6), IPv4 (len 4), Request who-has 10.10.30.2 tell 10.10.30.1, length 28
----- capture NEG30 vpc1 (0 frames)
----- capture NEG30 vpc2 (0 frames)
[sw2]$ ovs-ofctl --names dump-flows br0
 cookie=0x0, duration=21.938s, table=0, n_packets=9, n_bytes=800, priority=100,in_port=eth0,dl_vlan=10 actions=output:eth1
 cookie=0x0, duration=21.878s, table=0, n_packets=8, n_bytes=732, priority=100,in_port=eth0,dl_vlan=20 actions=output:eth1
 cookie=0x0, duration=21.829s, table=0, n_packets=8, n_bytes=704, priority=100,in_port=eth1,dl_vlan=10 actions=output:eth0
 cookie=0x0, duration=21.782s, table=0, n_packets=7, n_bytes=658, priority=100,in_port=eth1,dl_vlan=20 actions=output:eth0
 cookie=0x0, duration=21.736s, table=0, n_packets=16, n_bytes=1100, priority=0 actions=drop
(rc=0)
CHECK PASS: NEG VLAN 30 ping SW1 side -> router fails 
CHECK PASS: NEG VLAN 30 ping router -> SW1 side fails 
CHECK PASS: NEG VLAN 30 frames from SW1 reach SW2 eth0 (injection works) 
CHECK PASS: NEG VLAN 30 frames from router reach SW2 eth1 (injection works) 
CHECK PASS: NEG no VLAN 30 frame forwarded SW1 -> router (none on eth1 from SW1) 
CHECK PASS: NEG no VLAN 30 frame forwarded router -> SW1 (none on eth0 from router) 
CHECK PASS: NEG drop flow counter grew [12]

===== SUMMARY
PASS K2 vpc1: IP and gateway set
PASS K2 vpc2: IP and gateway set
PASS K3 tags/trunks set
PASS K3 patch ports present, no error
PASS K4 (EMULATED router) sub-interfaces 10.10.10.1 / 10.10.20.1
PASS K4 (EMULATED router) ip_forward=1
PASS K5 eth0 in br0
PASS K5 without SW2 flows (no NORMAL): VPC1->VPC2 fails, gateway not reachable
PASS K5 exactly 5 flows, no NORMAL
PASS K6 VPC1 -> gateway 10.10.10.1
PASS K6 VPC1 -> VPC2 ttl=63
PASS K6 VPC2 -> gateway 10.10.20.1
PASS K6 VPC2 -> VPC1 ttl=63
PASS K6 sw2eth0: VPC1->VPC2 request vlan 10 ttl 64 00:50:79:66:68:1c > router
PASS K6 sw2eth0: VPC1->VPC2 request vlan 20 ttl 63 router > 00:50:79:66:68:a7
PASS K6 sw2eth0: every frame is 802.1Q tagged (10 or 20)
PASS K6 sw2eth1: VPC1->VPC2 request vlan 10 ttl 64 00:50:79:66:68:1c > router
PASS K6 sw2eth1: VPC1->VPC2 request vlan 20 ttl 63 router > 00:50:79:66:68:a7
PASS K6 sw2eth1: every frame is 802.1Q tagged (10 or 20)
PASS K6 vpc1 cable: no 802.1Q frames
PASS K6 vpc2 cable: no 802.1Q frames
PASS K6 VPC2 cable: request from router MAC ttl 63
PASS K6 all four forwarding flows used
PASS NEG VLAN 30 ping SW1 side -> router fails
PASS NEG VLAN 30 ping router -> SW1 side fails
PASS NEG VLAN 30 frames from SW1 reach SW2 eth0 (injection works)
PASS NEG VLAN 30 frames from router reach SW2 eth1 (injection works)
PASS NEG no VLAN 30 frame forwarded SW1 -> router (none on eth1 from SW1)
PASS NEG no VLAN 30 frame forwarded router -> SW1 (none on eth0 from router)
PASS NEG drop flow counter grew
30/30 PASS
```
