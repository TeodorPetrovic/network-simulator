# Vežba 7 – test log (za održavanje, nije deo vežbe)

Testirano na pravim uređajima: `gns3/openvswitch` (OVS 2.17.12, bridge-evi br0..br3 sa `datapath_type=netdev`)
kao SW1 i SW2 (SW2 je na slici MikroTik), i pravi `vpcs` 0.8.5 iz simulatora
(`~/.local/share/network-simulator/bin/vpcs`, isti binarni fajl i iste opcije kao u `src/vpcs/runtime_impl.rs`:
`-p <konzola> -m <mac> -i 1 -F -R -s <lport> -c <rport> -t 127.0.0.1`) kao VPC1 i VPC2.
Imena kontejnera i mreža imaju prefiks `v7_`.

## Harness

- `src/test-setup.sh` – mreže `v7_n1` (SW1 eth0 – VPC1), `v7_n2` (SW1 eth1 – VPC2), `v7_n3` (SW1 eth2 – SW2 eth0).
  SW1 se pokreće sa `--entrypoint sleep` na n1, pa se n2 i n3 povezuju jedan po jedan (Docker broji ethN po redosledu
  povezivanja), zatim se `/etc/openvswitch/init.sh` pokreće ručno (stavlja sve ethN u br0, kao GNS3 pri startu).
  Docker IP adrese se brišu (`ip addr flush`).
- VPC: kontejner `v7_vpcN` (alpine, `--privileged`, IPv6 isključen) je samo "kabl". U njemu rade `vpcs` u UDP modu
  i `ubridge` iz simulatora (`source_udp = 20001:127.0.0.1:20000`, `destination_linux_raw = eth0`) – isti put kao u
  GNS3: VPCS UDP tunel -> ubridge -> Ethernet. glibc biblioteke hosta su montirane read-only (binarni fajlovi su
  dinamički linkovani). Startup skripta `set pcname VPCN`. MAC adresa VPCS-a je `00:50:79:66:68:XX`, XX = `-m`
  (simulator: poslednji bajt UUID-a čvora); bira se sa `V7_M1`/`V7_M2` (podrazumevano 28 i 167 = `1c`/`a7`, kao u rešenju).
- `src/run-test.py` – čita `<pre data-dev="...">` blokove iz `task/resenje.frag.html` i izvršava ih tačno kako su
  zapisani, redom: sw1/sw2 kroz `docker exec v7_swN bash -c` (komande prelomljene sa `\` šalju se kao jedna komanda
  sa prelomom), vpc1/vpc2 kroz telnet konzolu VPCS-a (`nc 127.0.0.1 5000` u kontejneru, čeka se prompt `VPCN> `).
  Posle `show` u Koraku 2 harness čita stvarne MAC adrese VPC-ova; ako se razlikuju od primera u rešenju, menja ih u
  kasnijim komandama (isto što student radi sa svojim MAC adresama). Tokom Koraka 4 i 8 tcpdump (`v7-tcpdump:local`
  = alpine + tcpdump) snima u mrežnom namespace-u SW2 eth0 (trunk kabl), VPC1 i VPC2 – ekvivalent Wireshark snimanja
  na kablu u GNS3. Posle svakog koraka automatske provere (PASS/FAIL).
- Dodatne negativne provere samo u harness-u (nisu u vežbi): VPC2 privremeno dobija `ip 10.10.10.99/24 10.10.10.1`
  i pinguje 10.10.10.1 (ARP za gateway VLAN-a 10 iz VLAN-a 20) -> `host (10.10.10.1) not reachable`; isto VPC1 sa
  10.10.20.98 -> 10.10.20.1; zatim se adrese vraćaju. Ping VPC1 -> VPC2 bez pravila za rutiranje -> `timeout`.
- `src/test-teardown.sh` – briše samo `v7_*` kontejnere i mreže, po tačnom imenu.

Pokretanje (u `src/`):

    ./test-teardown.sh; ./test-setup.sh && python3 run-test.py; ./test-teardown.sh
    V7_M1=21 V7_M2=179 ./test-setup.sh ...      # druge MAC adrese VPC-ova

## Rezultat

Četiri pokretanja od čistog stanja, svako 35/35 PASS:

| run | MAC VPC1 / VPC2 | SW2 eth0 rx / tx u Koraku 8 |
|-----|-----------------|-----------------------------|
| 1 | `00:50:79:66:68:1c` / `00:50:79:66:68:a7` (verbatim, kao u rešenju) | +6 / +6 |
| 2 | `00:50:79:66:68:15` / `00:50:79:66:68:b3` (zamena MAC adresa) | +8 / +6 |
| 3 | `00:50:79:66:68:c0` / `00:50:79:66:68:aa` (zamena MAC adresa) | +7 / +6 |
| 4 | `00:50:79:66:68:1c` / `00:50:79:66:68:a7` (konačni fragment, izlaz ispod) | +7 / +6 |

Ishodi identični; razlikuju se samo vremena i `rx` porta eth0 na SW2 (okviri koje SW2 primi i odbaci; `tx` uvek +6).

Provereno:
- `show` na VPCS-u prikazuje IP/masku, gateway i MAC; VPCS MAC se ne zadaje, čita se iz `show`;
- bez rutera ping ne radi: VPCS ispisuje `host (10.10.10.1) not reachable` (ARP za gateway bez odgovora, ICMP se ne šalje);
- ARP zahtev VPC1 prolazi br0 -> patch (trunk) -> br2 -> eth2 i na trunk kablu ima oznaku `vlan 10`; VPC2 -> `vlan 20`
  (VPCS u ARP zahtevu stavlja `ff:ff:ff:ff:ff:ff` kao ciljnu MAC, tcpdump: `who-has 10.10.10.1 (ff:ff:ff:ff:ff:ff) tell ...`);
- na kablovima ka VPC1/VPC2 nema nijednog 802.1Q okvira, a VPC1 ne vidi ništa iz VLAN-a 20 (i obrnuto);
- ARP za oba gateway-a odgovara OpenFlow (`show arp`: 02:00:00:00:10:01 i 02:00:00:00:20:01); ping gateway-a daje
  `timeout` (umesto `not reachable`); zahtev iz pogrešnog VLAN-a bez odgovora;
- posle pravila za rutiranje ping radi u oba smera sa `ttl=63`;
- `trace 10.10.20.12 -P 1`: skok 1 `* * *`, skok 2 `10.10.20.12`. OVS `dec_ttl` na paketu sa TTL 1 ne prosleđuje paket
  (packet-in kontroleru, kog nema) i ne šalje ICMP Time Exceeded, pa adresa gateway-a NE može da se vidi u skoku 1;
  pravilo za rutiranje ipak broji te pakete (brojač pravila VLAN 10 je za 3 veći od VLAN 20 posle trace-a);
- na trunk kablu svaki echo request se vidi dvaput (isti ICMP id/seq i IP id): vlan 10 / ttl 64 (VPC1 MAC -> GW10 MAC)
  ka SW2, pa vlan 20 / ttl 63 (GW20 MAC -> VPC2 MAC) od SW2; odgovori obrnuto; brojači pravila za rutiranje +3 svaki.

Činjenice iz ranijih testova (alpine, koriste se u uvodu, nepromenjene):
- `output:eth0` za paket koji je ušao na eth0 se tiho odbacuje; `in_port` i `output:in_port` rade isto (`IN_PORT`).
- `tag=99` ili `trunks=30` na SW2 eth0 ne menja ništa za OpenFlow pravila – VLAN podešavanja porta koristi samo NORMAL.
- `vlan_tci=0x1000/0x1000`, `vlan_tci=0x0000/0x1000`, `strip_vlan`, `mod_vlan_vid` na neoznačenom okviru – sintaksa prihvaćena.
- Komanda prelomljena sa `\` unutar dvostrukih navodnika, sa uvlačenjem nastavka sa 2 razmaka, radi u `sh` i `bash`.
- Patch par između br0 i br2 radi bez greške jer su oba bridge-a `netdev`.

## Pun izlaz poslednjeg pokretanja (run4, nad konačnim fragmentom)

```

##### Korak 2 – IP adrese, podrazumevani izlaz i MAC adrese VPC-ova
[VPC1> ip 10.10.10.11/24 10.10.10.1
Checking for duplicate address...
VPC1 : 10.10.10.11 255.255.255.0 gateway 10.10.10.1
(rc=0)
[VPC1> show
NAME   IP/MASK              GATEWAY           MAC                LPORT  RHOST:PORT
VPC1   10.10.10.11/24       10.10.10.1        00:50:79:66:68:1c  20000  127.0.0.1:20001
       fe80::250:79ff:fe66:681c/64
(rc=0)
[VPC2> ip 10.10.20.12/24 10.10.20.1
Checking for duplicate address...
VPC2 : 10.10.20.12 255.255.255.0 gateway 10.10.20.1
(rc=0)
[VPC2> show
NAME   IP/MASK              GATEWAY           MAC                LPORT  RHOST:PORT
VPC2   10.10.20.12/24       10.10.20.1        00:50:79:66:68:a7  20000  127.0.0.1:20001
       fe80::250:79ff:fe66:68a7/64
(rc=0)
CHECK PASS: K2 vpc1 show: IP, gateway, MAC VPC1   10.10.10.11/24       10.10.10.1        00:50:79:66:68:1c
CHECK PASS: K2 vpc2 show: IP, gateway, MAC VPC2   10.10.20.12/24       10.10.20.1        00:50:79:66:68:a7
MAC adrese VPC-ova (iz show): {'vpc1': '00:50:79:66:68:1c', 'vpc2': '00:50:79:66:68:a7'} | primer u resenju: {'vpc1': '00:50:79:66:68:1c', 'vpc2': '00:50:79:66:68:a7'}

##### Korak 3 – bridge-evi i VLAN uloge portova na SW1
[sw1]$ ovs-vsctl show
d4c98374-9fcc-47a3-9af0-ffb452cec613
    Bridge br0
        datapath_type: netdev
        Port eth2
            Interface eth2
        Port eth1
            Interface eth1
        Port eth0
            Interface eth0
        Port br0
            Interface br0
                type: internal
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
    Bridge br2
        datapath_type: netdev
        Port br2
            Interface br2
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
d4c98374-9fcc-47a3-9af0-ffb452cec613
    Bridge br0
        datapath_type: netdev
        Port eth0
            tag: 10
            Interface eth0
        Port br0
            Interface br0
                type: internal
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
    Bridge br2
        datapath_type: netdev
        Port br2
            Interface br2
                type: internal
        Port eth2
            trunks: [10, 20]
            Interface eth2
        Port eth1
            tag: 20
            Interface eth1
(rc=0)
[VPC1> ping 10.10.20.12
host (10.10.10.1) not reachable
(rc=0)
CHECK PASS: K3 VPC1->VPC2 ping fails: gateway not reachable (no router) 
CHECK PASS: K3 tags/trunks set 

##### Korak 4 – patch portovi kao trunk između br0 i br2
[sw1]$ ovs-vsctl add-port br0 br0-br2 trunks=10,20 \
  -- set interface br0-br2 type=patch options:peer=br2-br0
(rc=0)
[sw1]$ ovs-vsctl add-port br2 br2-br0 trunks=10,20 \
  -- set interface br2-br0 type=patch options:peer=br0-br2
(rc=0)
[sw1]$ ovs-vsctl show
d4c98374-9fcc-47a3-9af0-ffb452cec613
    Bridge br0
        datapath_type: netdev
        Port br0-br2
            trunks: [10, 20]
            Interface br0-br2
                type: patch
                options: {peer=br2-br0}
        Port eth0
            tag: 10
            Interface eth0
        Port br0
            Interface br0
                type: internal
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
    Bridge br2
        datapath_type: netdev
        Port br2
            Interface br2
                type: internal
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
(rc=0)
[VPC1> ping 10.10.20.12
host (10.10.10.1) not reachable
(rc=0)
[VPC2> ping 10.10.10.11
host (10.10.20.1) not reachable
(rc=0)
----- capture K4 sw2eth0 (6 frames)
  12:11:55.944641 00:50:79:66:68:1c > ff:ff:ff:ff:ff:ff, ethertype 802.1Q (0x8100), length 68: vlan 10, p 0, ethertype ARP (0x0806), Ethernet (len 6), IPv4 (len 4), Request who-has 10.10.10.1 (ff:ff:ff:ff:ff:ff) tell 10.10.10.11, length 50
  12:11:56.945830 00:50:79:66:68:1c > ff:ff:ff:ff:ff:ff, ethertype 802.1Q (0x8100), length 68: vlan 10, p 0, ethertype ARP (0x0806), Ethernet (len 6), IPv4 (len 4), Request who-has 10.10.10.1 (ff:ff:ff:ff:ff:ff) tell 10.10.10.11, length 50
  12:11:57.946752 00:50:79:66:68:1c > ff:ff:ff:ff:ff:ff, ethertype 802.1Q (0x8100), length 68: vlan 10, p 0, ethertype ARP (0x0806), Ethernet (len 6), IPv4 (len 4), Request who-has 10.10.10.1 (ff:ff:ff:ff:ff:ff) tell 10.10.10.11, length 50
  12:11:58.948984 00:50:79:66:68:a7 > ff:ff:ff:ff:ff:ff, ethertype 802.1Q (0x8100), length 68: vlan 20, p 0, ethertype ARP (0x0806), Ethernet (len 6), IPv4 (len 4), Request who-has 10.10.20.1 (ff:ff:ff:ff:ff:ff) tell 10.10.20.12, length 50
  12:11:59.949479 00:50:79:66:68:a7 > ff:ff:ff:ff:ff:ff, ethertype 802.1Q (0x8100), length 68: vlan 20, p 0, ethertype ARP (0x0806), Ethernet (len 6), IPv4 (len 4), Request who-has 10.10.20.1 (ff:ff:ff:ff:ff:ff) tell 10.10.20.12, length 50
  12:12:00.949813 00:50:79:66:68:a7 > ff:ff:ff:ff:ff:ff, ethertype 802.1Q (0x8100), length 68: vlan 20, p 0, ethertype ARP (0x0806), Ethernet (len 6), IPv4 (len 4), Request who-has 10.10.20.1 (ff:ff:ff:ff:ff:ff) tell 10.10.20.12, length 50
----- capture K4 vpc1 (3 frames)
  12:11:55.944378 00:50:79:66:68:1c > ff:ff:ff:ff:ff:ff, ethertype ARP (0x0806), length 64: Ethernet (len 6), IPv4 (len 4), Request who-has 10.10.10.1 (ff:ff:ff:ff:ff:ff) tell 10.10.10.11, length 50
  12:11:56.945518 00:50:79:66:68:1c > ff:ff:ff:ff:ff:ff, ethertype ARP (0x0806), length 64: Ethernet (len 6), IPv4 (len 4), Request who-has 10.10.10.1 (ff:ff:ff:ff:ff:ff) tell 10.10.10.11, length 50
  12:11:57.946538 00:50:79:66:68:1c > ff:ff:ff:ff:ff:ff, ethertype ARP (0x0806), length 64: Ethernet (len 6), IPv4 (len 4), Request who-has 10.10.10.1 (ff:ff:ff:ff:ff:ff) tell 10.10.10.11, length 50
----- capture K4 vpc2 (3 frames)
  12:11:58.948538 00:50:79:66:68:a7 > ff:ff:ff:ff:ff:ff, ethertype ARP (0x0806), length 64: Ethernet (len 6), IPv4 (len 4), Request who-has 10.10.20.1 (ff:ff:ff:ff:ff:ff) tell 10.10.20.12, length 50
  12:11:59.948989 00:50:79:66:68:a7 > ff:ff:ff:ff:ff:ff, ethertype ARP (0x0806), length 64: Ethernet (len 6), IPv4 (len 4), Request who-has 10.10.20.1 (ff:ff:ff:ff:ff:ff) tell 10.10.20.12, length 50
  12:12:00.949335 00:50:79:66:68:a7 > ff:ff:ff:ff:ff:ff, ethertype ARP (0x0806), length 64: Ethernet (len 6), IPv4 (len 4), Request who-has 10.10.20.1 (ff:ff:ff:ff:ff:ff) tell 10.10.20.12, length 50
CHECK PASS: K4 patch ports present, no error 
CHECK PASS: K4 VPC1 ping still fails 
CHECK PASS: K4 VPC2 ping still fails 
CHECK PASS: K4 trunk: VPC1 ARP arrives tagged vlan 10 (via patch) 
CHECK PASS: K4 trunk: VPC2 ARP arrives tagged vlan 20 
CHECK PASS: K4 VPC1 link: ARP seen untagged 
CHECK PASS: K4 VPC1 link: no 802.1Q frames 
CHECK PASS: K4 VPC2 link: no 802.1Q frames 
CHECK PASS: K4 VPC1 link: nothing from VLAN 20 
CHECK PASS: K4 VPC2 link: nothing from VLAN 10 

##### Korak 5 – SW2 bez NORMAL pravila
[sw2]$ ovs-vsctl port-to-br eth0
br0
(rc=0)
[sw2]$ ovs-ofctl dump-flows br0
NXST_FLOW reply (xid=0x4):
 cookie=0x0, duration=30.658s, table=0, n_packets=42, n_bytes=3088, idle_age=2, priority=0 actions=NORMAL
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
[VPC1> ping 10.10.10.1 -c 1
10.10.10.1 icmp_seq=1 timeout
(rc=0)
[VPC1> show arp
02:00:00:00:10:01  10.10.10.1 expires in 118 seconds
(rc=0)
[VPC2> ping 10.10.20.1 -c 1
10.10.20.1 icmp_seq=1 timeout
(rc=0)
[VPC2> show arp
02:00:00:00:20:01  10.10.20.1 expires in 118 seconds
(rc=0)
CHECK PASS: K6 VPC1 gateway ping: ARP ok, ICMP timeout 
CHECK PASS: K6 VPC2 gateway ping: ARP ok, ICMP timeout 
CHECK PASS: K6 VPC1 learned GW10 MAC from OpenFlow 
CHECK PASS: K6 VPC2 learned GW20 MAC from OpenFlow 
[VPC2> ip 10.10.10.99/24 10.10.10.1
Checking for duplicate address...
VPC2 : 10.10.10.99 255.255.255.0 gateway 10.10.10.1
(rc=0)
[VPC2> ping 10.10.10.1 -c 1
host (10.10.10.1) not reachable
(rc=0)
CHECK PASS: K6 NEG: ARP for 10.10.10.1 asked from VLAN 20 is NOT answered 
[VPC2> ip 10.10.20.12/24 10.10.20.1
Checking for duplicate address...
VPC2 : 10.10.20.12 255.255.255.0 gateway 10.10.20.1
(rc=0)
[VPC1> ip 10.10.20.98/24 10.10.20.1
Checking for duplicate address...
VPC1 : 10.10.20.98 255.255.255.0 gateway 10.10.20.1
(rc=0)
[VPC1> ping 10.10.20.1 -c 1
host (10.10.20.1) not reachable
(rc=0)
CHECK PASS: K6 NEG: ARP for 10.10.20.1 asked from VLAN 10 is NOT answered 
[VPC1> ip 10.10.10.11/24 10.10.10.1
Checking for duplicate address...
VPC1 : 10.10.10.11 255.255.255.0 gateway 10.10.10.1
(rc=0)
[VPC1> ping 10.10.20.12 -c 2
10.10.20.12 icmp_seq=1 timeout
10.10.20.12 icmp_seq=2 timeout
(rc=0)
CHECK PASS: K6 NEG: VPC1->VPC2 still fails without routing flows 

##### Korak 7 – rutiranje između VLAN-ova
[sw2]$ ovs-ofctl add-flow br0 "priority=100,in_port=eth0,dl_vlan=10,ip,nw_dst=10.10.20.0/24,\
  actions=mod_vlan_vid:20,mod_dl_src:02:00:00:00:20:01,mod_dl_dst:00:50:79:66:68:a7,dec_ttl,in_port"
(rc=0)
[sw2]$ ovs-ofctl add-flow br0 "priority=100,in_port=eth0,dl_vlan=20,ip,nw_dst=10.10.10.0/24,\
  actions=mod_vlan_vid:10,mod_dl_src:02:00:00:00:10:01,mod_dl_dst:00:50:79:66:68:1c,dec_ttl,in_port"
(rc=0)
[VPC1> ping 10.10.20.12 -c 2
84 bytes from 10.10.20.12 icmp_seq=1 ttl=63 time=2.569 ms
84 bytes from 10.10.20.12 icmp_seq=2 ttl=63 time=2.767 ms
(rc=0)
[VPC2> ping 10.10.10.11 -c 2
84 bytes from 10.10.10.11 icmp_seq=1 ttl=63 time=2.537 ms
84 bytes from 10.10.10.11 icmp_seq=2 ttl=63 time=2.813 ms
(rc=0)
[VPC1> trace 10.10.20.12 -P 1
trace to 10.10.20.12, 8 hops max (ICMP), press Ctrl+C to stop
 1     *  *  *
 2   10.10.20.12   2.980 ms  2.631 ms  2.423 ms
(rc=0)
CHECK PASS: K7 ping works, ttl=63 
CHECK PASS: K7 ping works, ttl=63 
CHECK PASS: K7 trace: hop 1 = router (no ICMP time exceeded), hop 2 = VPC2 

##### Korak 8 – dokaz da saobraćaj ide preko SW2
[sw2]$ ovs-ofctl --names dump-ports br0 eth0
OFPST_PORT reply (xid=0x4): 1 ports
  port  eth0: rx pkts=93, bytes=7258, drop=0, errs=0, frame=0, over=0, crc=0
           tx pkts=27, bytes=2394, drop=0, errs=0, coll=0
(rc=0)
[sw2]$ ovs-ofctl --names dump-flows br0
 cookie=0x0, duration=35.424s, table=0, n_packets=1, n_bytes=68, priority=200,arp,in_port=eth0,dl_vlan=10,arp_tpa=10.10.10.1,arp_op=1 actions=move:NXM_OF_ETH_SRC[]->NXM_OF_ETH_DST[],mod_dl_src:02:00:00:00:10:01,load:0x2->NXM_OF_ARP_OP[],move:NXM_NX_ARP_SHA[]->NXM_NX_ARP_THA[],move:NXM_OF_ARP_SPA[]->NXM_OF_ARP_TPA[],load:0x20000001001->NXM_NX_ARP_SHA[],load:0xa0a0a01->NXM_OF_ARP_SPA[],IN_PORT
 cookie=0x0, duration=35.374s, table=0, n_packets=1, n_bytes=68, priority=200,arp,in_port=eth0,dl_vlan=20,arp_tpa=10.10.20.1,arp_op=1 actions=move:NXM_OF_ETH_SRC[]->NXM_OF_ETH_DST[],mod_dl_src:02:00:00:00:20:01,load:0x2->NXM_OF_ARP_OP[],move:NXM_NX_ARP_SHA[]->NXM_NX_ARP_THA[],move:NXM_OF_ARP_SPA[]->NXM_OF_ARP_TPA[],load:0x20000002001->NXM_NX_ARP_SHA[],load:0xa0a1401->NXM_OF_ARP_SPA[],IN_PORT
 cookie=0x0, duration=9.189s, table=0, n_packets=10, n_bytes=1068, priority=100,ip,in_port=eth0,dl_vlan=10,nw_dst=10.10.20.0/24 actions=mod_vlan_vid:20,mod_dl_src:02:00:00:00:20:01,mod_dl_dst:00:50:79:66:68:a7,dec_ttl,IN_PORT
 cookie=0x0, duration=9.138s, table=0, n_packets=7, n_bytes=738, priority=100,ip,in_port=eth0,dl_vlan=20,nw_dst=10.10.10.0/24 actions=mod_vlan_vid:10,mod_dl_src:02:00:00:00:10:01,mod_dl_dst:00:50:79:66:68:1c,dec_ttl,IN_PORT
(rc=0)
[VPC1> ping 10.10.20.12 -c 3
84 bytes from 10.10.20.12 icmp_seq=1 ttl=63 time=2.707 ms
84 bytes from 10.10.20.12 icmp_seq=2 ttl=63 time=2.689 ms
84 bytes from 10.10.20.12 icmp_seq=3 ttl=63 time=2.424 ms
(rc=0)
[sw2]$ ovs-ofctl --names dump-ports br0 eth0
OFPST_PORT reply (xid=0x4): 1 ports
  port  eth0: rx pkts=100, bytes=7916, drop=0, errs=0, frame=0, over=0, crc=0
           tx pkts=33, bytes=3006, drop=0, errs=0, coll=0
(rc=0)
[sw2]$ ovs-ofctl --names dump-flows br0
 cookie=0x0, duration=38.541s, table=0, n_packets=1, n_bytes=68, priority=200,arp,in_port=eth0,dl_vlan=10,arp_tpa=10.10.10.1,arp_op=1 actions=move:NXM_OF_ETH_SRC[]->NXM_OF_ETH_DST[],mod_dl_src:02:00:00:00:10:01,load:0x2->NXM_OF_ARP_OP[],move:NXM_NX_ARP_SHA[]->NXM_NX_ARP_THA[],move:NXM_OF_ARP_SPA[]->NXM_OF_ARP_TPA[],load:0x20000001001->NXM_NX_ARP_SHA[],load:0xa0a0a01->NXM_OF_ARP_SPA[],IN_PORT
 cookie=0x0, duration=38.491s, table=0, n_packets=1, n_bytes=68, priority=200,arp,in_port=eth0,dl_vlan=20,arp_tpa=10.10.20.1,arp_op=1 actions=move:NXM_OF_ETH_SRC[]->NXM_OF_ETH_DST[],mod_dl_src:02:00:00:00:20:01,load:0x2->NXM_OF_ARP_OP[],move:NXM_NX_ARP_SHA[]->NXM_NX_ARP_THA[],move:NXM_OF_ARP_SPA[]->NXM_OF_ARP_TPA[],load:0x20000002001->NXM_NX_ARP_SHA[],load:0xa0a1401->NXM_OF_ARP_SPA[],IN_PORT
 cookie=0x0, duration=12.306s, table=0, n_packets=13, n_bytes=1374, priority=100,ip,in_port=eth0,dl_vlan=10,nw_dst=10.10.20.0/24 actions=mod_vlan_vid:20,mod_dl_src:02:00:00:00:20:01,mod_dl_dst:00:50:79:66:68:a7,dec_ttl,IN_PORT
 cookie=0x0, duration=12.255s, table=0, n_packets=10, n_bytes=1044, priority=100,ip,in_port=eth0,dl_vlan=20,nw_dst=10.10.10.0/24 actions=mod_vlan_vid:10,mod_dl_src:02:00:00:00:10:01,mod_dl_dst:00:50:79:66:68:1c,dec_ttl,IN_PORT
(rc=0)
----- capture K8 sw2eth0 (12 frames)
  12:12:39.005567 00:50:79:66:68:1c > 02:00:00:00:10:01, ethertype 802.1Q (0x8100), length 102: vlan 10, p 0, ethertype IPv4 (0x0800), (tos 0x0, ttl 64, id 38455, offset 0, flags [none], proto ICMP (1), length 84) | 10.10.10.11 > 10.10.20.12: ICMP echo request, id 14230, seq 1, length 64
  12:12:39.005823 02:00:00:00:20:01 > 00:50:79:66:68:a7, ethertype 802.1Q (0x8100), length 102: vlan 20, p 0, ethertype IPv4 (0x0800), (tos 0x0, ttl 63, id 38455, offset 0, flags [none], proto ICMP (1), length 84) | 10.10.10.11 > 10.10.20.12: ICMP echo request, id 14230, seq 1, length 64
  12:12:39.007053 00:50:79:66:68:a7 > 02:00:00:00:20:01, ethertype 802.1Q (0x8100), length 102: vlan 20, p 0, ethertype IPv4 (0x0800), (tos 0x0, ttl 64, id 38455, offset 0, flags [none], proto ICMP (1), length 84) | 10.10.20.12 > 10.10.10.11: ICMP echo reply, id 14230, seq 1, length 64
  12:12:39.007294 02:00:00:00:10:01 > 00:50:79:66:68:1c, ethertype 802.1Q (0x8100), length 102: vlan 10, p 0, ethertype IPv4 (0x0800), (tos 0x0, ttl 63, id 38455, offset 0, flags [none], proto ICMP (1), length 84) | 10.10.20.12 > 10.10.10.11: ICMP echo reply, id 14230, seq 1, length 64
  12:12:40.008950 00:50:79:66:68:1c > 02:00:00:00:10:01, ethertype 802.1Q (0x8100), length 102: vlan 10, p 0, ethertype IPv4 (0x0800), (tos 0x0, ttl 64, id 38456, offset 0, flags [none], proto ICMP (1), length 84) | 10.10.10.11 > 10.10.20.12: ICMP echo request, id 14486, seq 2, length 64
  12:12:40.009248 02:00:00:00:20:01 > 00:50:79:66:68:a7, ethertype 802.1Q (0x8100), length 102: vlan 20, p 0, ethertype IPv4 (0x0800), (tos 0x0, ttl 63, id 38456, offset 0, flags [none], proto ICMP (1), length 84) | 10.10.10.11 > 10.10.20.12: ICMP echo request, id 14486, seq 2, length 64
  12:12:40.010343 00:50:79:66:68:a7 > 02:00:00:00:20:01, ethertype 802.1Q (0x8100), length 102: vlan 20, p 0, ethertype IPv4 (0x0800), (tos 0x0, ttl 64, id 38456, offset 0, flags [none], proto ICMP (1), length 84) | 10.10.20.12 > 10.10.10.11: ICMP echo reply, id 14486, seq 2, length 64
  12:12:40.010712 02:00:00:00:10:01 > 00:50:79:66:68:1c, ethertype 802.1Q (0x8100), length 102: vlan 10, p 0, ethertype IPv4 (0x0800), (tos 0x0, ttl 63, id 38456, offset 0, flags [none], proto ICMP (1), length 84) | 10.10.20.12 > 10.10.10.11: ICMP echo reply, id 14486, seq 2, length 64
  12:12:41.013428 00:50:79:66:68:1c > 02:00:00:00:10:01, ethertype 802.1Q (0x8100), length 102: vlan 10, p 0, ethertype IPv4 (0x0800), (tos 0x0, ttl 64, id 38457, offset 0, flags [none], proto ICMP (1), length 84) | 10.10.10.11 > 10.10.20.12: ICMP echo request, id 14742, seq 3, length 64
  12:12:41.013733 02:00:00:00:20:01 > 00:50:79:66:68:a7, ethertype 802.1Q (0x8100), length 102: vlan 20, p 0, ethertype IPv4 (0x0800), (tos 0x0, ttl 63, id 38457, offset 0, flags [none], proto ICMP (1), length 84) | 10.10.10.11 > 10.10.20.12: ICMP echo request, id 14742, seq 3, length 64
  12:12:41.014681 00:50:79:66:68:a7 > 02:00:00:00:20:01, ethertype 802.1Q (0x8100), length 102: vlan 20, p 0, ethertype IPv4 (0x0800), (tos 0x0, ttl 64, id 38457, offset 0, flags [none], proto ICMP (1), length 84) | 10.10.20.12 > 10.10.10.11: ICMP echo reply, id 14742, seq 3, length 64
  12:12:41.014912 02:00:00:00:10:01 > 00:50:79:66:68:1c, ethertype 802.1Q (0x8100), length 102: vlan 10, p 0, ethertype IPv4 (0x0800), (tos 0x0, ttl 63, id 38457, offset 0, flags [none], proto ICMP (1), length 84) | 10.10.20.12 > 10.10.10.11: ICMP echo reply, id 14742, seq 3, length 64
----- capture K8 vpc1 (6 frames)
  12:12:39.005323 00:50:79:66:68:1c > 02:00:00:00:10:01, ethertype IPv4 (0x0800), length 98: (tos 0x0, ttl 64, id 38455, offset 0, flags [none], proto ICMP (1), length 84) | 10.10.10.11 > 10.10.20.12: ICMP echo request, id 14230, seq 1, length 64
  12:12:39.007785 02:00:00:00:10:01 > 00:50:79:66:68:1c, ethertype IPv4 (0x0800), length 98: (tos 0x0, ttl 63, id 38455, offset 0, flags [none], proto ICMP (1), length 84) | 10.10.20.12 > 10.10.10.11: ICMP echo reply, id 14230, seq 1, length 64
  12:12:40.008690 00:50:79:66:68:1c > 02:00:00:00:10:01, ethertype IPv4 (0x0800), length 98: (tos 0x0, ttl 64, id 38456, offset 0, flags [none], proto ICMP (1), length 84) | 10.10.10.11 > 10.10.20.12: ICMP echo request, id 14486, seq 2, length 64
  12:12:40.011192 02:00:00:00:10:01 > 00:50:79:66:68:1c, ethertype IPv4 (0x0800), length 98: (tos 0x0, ttl 63, id 38456, offset 0, flags [none], proto ICMP (1), length 84) | 10.10.20.12 > 10.10.10.11: ICMP echo reply, id 14486, seq 2, length 64
  12:12:41.013012 00:50:79:66:68:1c > 02:00:00:00:10:01, ethertype IPv4 (0x0800), length 98: (tos 0x0, ttl 64, id 38457, offset 0, flags [none], proto ICMP (1), length 84) | 10.10.10.11 > 10.10.20.12: ICMP echo request, id 14742, seq 3, length 64
  12:12:41.015145 02:00:00:00:10:01 > 00:50:79:66:68:1c, ethertype IPv4 (0x0800), length 98: (tos 0x0, ttl 63, id 38457, offset 0, flags [none], proto ICMP (1), length 84) | 10.10.20.12 > 10.10.10.11: ICMP echo reply, id 14742, seq 3, length 64
----- capture K8 vpc2 (6 frames)
  12:12:39.006102 02:00:00:00:20:01 > 00:50:79:66:68:a7, ethertype IPv4 (0x0800), length 98: (tos 0x0, ttl 63, id 38455, offset 0, flags [none], proto ICMP (1), length 84) | 10.10.10.11 > 10.10.20.12: ICMP echo request, id 14230, seq 1, length 64
  12:12:39.006382 00:50:79:66:68:a7 > 02:00:00:00:20:01, ethertype IPv4 (0x0800), length 98: (tos 0x0, ttl 64, id 38455, offset 0, flags [none], proto ICMP (1), length 84) | 10.10.20.12 > 10.10.10.11: ICMP echo reply, id 14230, seq 1, length 64
  12:12:40.009560 02:00:00:00:20:01 > 00:50:79:66:68:a7, ethertype IPv4 (0x0800), length 98: (tos 0x0, ttl 63, id 38456, offset 0, flags [none], proto ICMP (1), length 84) | 10.10.10.11 > 10.10.20.12: ICMP echo request, id 14486, seq 2, length 64
  12:12:40.009964 00:50:79:66:68:a7 > 02:00:00:00:20:01, ethertype IPv4 (0x0800), length 98: (tos 0x0, ttl 64, id 38456, offset 0, flags [none], proto ICMP (1), length 84) | 10.10.20.12 > 10.10.10.11: ICMP echo reply, id 14486, seq 2, length 64
  12:12:41.013985 02:00:00:00:20:01 > 00:50:79:66:68:a7, ethertype IPv4 (0x0800), length 98: (tos 0x0, ttl 63, id 38457, offset 0, flags [none], proto ICMP (1), length 84) | 10.10.10.11 > 10.10.20.12: ICMP echo request, id 14742, seq 3, length 64
  12:12:41.014286 00:50:79:66:68:a7 > 02:00:00:00:20:01, ethertype IPv4 (0x0800), length 98: (tos 0x0, ttl 64, id 38457, offset 0, flags [none], proto ICMP (1), length 84) | 10.10.20.12 > 10.10.10.11: ICMP echo reply, id 14742, seq 3, length 64
port eth0 delta rx 7 tx 6 | flow deltas {'priority=200,arp,in_port=eth0,dl_vlan=10,arp_tpa=10.10.10.1,arp_op=1': 0, 'priority=200,arp,in_port=eth0,dl_vlan=20,arp_tpa=10.10.20.1,arp_op=1': 0, 'priority=100,ip,in_port=eth0,dl_vlan=10,nw_dst=10.10.20.0/24': 3, 'priority=100,ip,in_port=eth0,dl_vlan=20,nw_dst=10.10.10.0/24': 3}
CHECK PASS: K8 eth0 rx and tx both grew by >= 6 rx+7 tx+6
CHECK PASS: K8 routing flow counters +3 each [3] [3]
CHECK PASS: K8 ping ok (3 replies ttl=63) 
CHECK PASS: K8 trunk: every request seen twice: vlan10/ttl64 in, vlan20/ttl63 out 3 requests
CHECK PASS: K8 trunk: replies vlan20/ttl64 in, vlan10/ttl63 out 
CHECK PASS: K8 VPC1 link: no 802.1Q 
CHECK PASS: K8 VPC2 link: no 802.1Q 
CHECK PASS: K8 VPC2 sees request from GW20 MAC with ttl 63 
CHECK PASS: K8 VPC1 sends request to GW10 MAC with ttl 64 

===== SUMMARY
PASS K2 vpc1 show: IP, gateway, MAC
PASS K2 vpc2 show: IP, gateway, MAC
PASS K3 VPC1->VPC2 ping fails: gateway not reachable (no router)
PASS K3 tags/trunks set
PASS K4 patch ports present, no error
PASS K4 VPC1 ping still fails
PASS K4 VPC2 ping still fails
PASS K4 trunk: VPC1 ARP arrives tagged vlan 10 (via patch)
PASS K4 trunk: VPC2 ARP arrives tagged vlan 20
PASS K4 VPC1 link: ARP seen untagged
PASS K4 VPC1 link: no 802.1Q frames
PASS K4 VPC2 link: no 802.1Q frames
PASS K4 VPC1 link: nothing from VLAN 20
PASS K4 VPC2 link: nothing from VLAN 10
PASS K5 NORMAL deleted
PASS K5 eth0 in br0
PASS K6 VPC1 gateway ping: ARP ok, ICMP timeout
PASS K6 VPC2 gateway ping: ARP ok, ICMP timeout
PASS K6 VPC1 learned GW10 MAC from OpenFlow
PASS K6 VPC2 learned GW20 MAC from OpenFlow
PASS K6 NEG: ARP for 10.10.10.1 asked from VLAN 20 is NOT answered
PASS K6 NEG: ARP for 10.10.20.1 asked from VLAN 10 is NOT answered
PASS K6 NEG: VPC1->VPC2 still fails without routing flows
PASS K7 ping works, ttl=63
PASS K7 ping works, ttl=63
PASS K7 trace: hop 1 = router (no ICMP time exceeded), hop 2 = VPC2
PASS K8 eth0 rx and tx both grew by >= 6
PASS K8 routing flow counters +3 each
PASS K8 ping ok (3 replies ttl=63)
PASS K8 trunk: every request seen twice: vlan10/ttl64 in, vlan20/ttl63 out
PASS K8 trunk: replies vlan20/ttl64 in, vlan10/ttl63 out
PASS K8 VPC1 link: no 802.1Q
PASS K8 VPC2 link: no 802.1Q
PASS K8 VPC2 sees request from GW20 MAC with ttl 63
PASS K8 VPC1 sends request to GW10 MAC with ttl 64
35/35 PASS
```
