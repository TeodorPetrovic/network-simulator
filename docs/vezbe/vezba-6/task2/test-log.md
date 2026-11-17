# Test log - Vezba 6, Zadatak 2 (samo za odrzavaoca)

Okruzenje: Docker, 3 kontejnera (v6t2_ovs = gns3/openvswitch, OVS 2.17, netdev datapath; v6t2_in = PC, v6t2_out = SERVER, alpine), 2 docker mreze v6t2_n1, v6t2_n2 (jedna mreza = jedan kabl; n1 = eth0, n2 = eth1 na OVS). Kontejneri pokrenuti sa `sleep infinity`, mreze povezane jedna po jedna, zatim `/etc/openvswitch/init.sh`. Harness: `ethtool -K eth0 tx off` na PC i SERVER (nije u vezbi). Komande resenja izvrsene skriptom koja cita `<pre data-dev>` blokove iz resenje.frag.html, tacno kako su zapisane. Pun redosled izvrsen vise puta od cistog stanja (run1, run2, runA, runB); ishodi isti, brojaci pocetnih i konacnih koraka identicni (ping/wget rezultati, new=3, drop eth1=6, na kraju 1/1/12).

## Preliminarna provera conntrack-a
- `ct(table=1)` + `ct(commit)` rade u ovom datapath-u (userspace conntrack): inside->outside ping i HTTP prolaze, odgovori prolaze kao +est, outside->inside +new se odbacuje. Radi i bez `-O OpenFlow13` (OF1.0 sa NXM ct).
- `ovs-appctl dpctl/dump-conntrack` u ovoj slici: "multiple datapaths, specify one", a sa `netdev@ovs-netdev` ili `ovs-netdev` "datapath not found" - ne moze se koristiti, ali ponasanje dokazuje conntrack. U vezbi se ne koristi.
- `del-flows main-net 'table=1,ct_state=+new+trk,in_port=eth0,ip'` brise samo to pravilo (ostala imaju drugo stanje/port).
- `+inv` pravilo: u svakom pokretanju 2 paketa (74 B) u koracima 7-8 i 1 u koraku 10; utvrdjeno privremenim pravilima da je to SYN-ACK sa eth1 proglasen nevazecim (+inv) u prvim vezama; wget ga ponovi, pa ishod ne trpi. Ne gura se u tekst vise od "obicno 0 ili mali broj".
- Zbir brojaca tabele 1 jednak brojacu `ip actions=ct(table=1)` u tabeli 0 (38 = 2+3+14+13+6+0).

## Kompletan izlaz izvrsavanja (run2.txt)
```

## Korak 1 – Topologija

## Korak 2 – IP adrese i HTTP serveri
[pc]$ ip link set eth0 up
(rc=0)
[pc]$ ip addr add 10.17.0.11/24 dev eth0
(rc=0)
[pc]$ echo '#!/bin/sh' > /srv.sh
(rc=0)
[pc]$ echo 'while read l; do [ "${#l}" -le 1 ] && break; done' >> /srv.sh
(rc=0)
[pc]$ echo 'printf "HTTP/1.0 200 OK\r\nContent-Length: 10\r\n\r\nINSIDE OK\n"' >> /srv.sh
(rc=0)
[pc]$ chmod +x /srv.sh
(rc=0)
[pc]$ nc -lk -p 80 -e /srv.sh &
[srv]$ ip link set eth0 up
(rc=0)
[srv]$ ip addr add 10.17.0.100/24 dev eth0
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
[srv]$ nc -lk -p 8080 -e /srv.sh &

## Korak 3 – Bridge main-net bez pravila
[ovs1]$ ovs-vsctl del-port br0 eth0
(rc=0)
[ovs1]$ ovs-vsctl del-port br0 eth1
(rc=0)
[ovs1]$ ovs-vsctl add-br main-net
(rc=0)
[ovs1]$ ovs-vsctl add-port main-net eth0
(rc=0)
[ovs1]$ ovs-vsctl add-port main-net eth1
(rc=0)
[ovs1]$ ovs-ofctl del-flows main-net
(rc=0)
[ovs1]$ ovs-ofctl dump-flows main-net
NXST_FLOW reply (xid=0x4):
(rc=0)
[pc]$ ping -c 3 -W 1 10.17.0.100
PING 10.17.0.100 (10.17.0.100): 56 data bytes

--- 10.17.0.100 ping statistics ---
3 packets transmitted, 0 packets received, 100% packet loss
(rc=1)

## Korak 4 – Prosleđivanje bez zaštite
[ovs1]$ ovs-ofctl add-flow main-net 'priority=100,arp,actions=flood'
(rc=0)
[ovs1]$ ovs-ofctl add-flow main-net 'priority=10,in_port=eth0,actions=output:eth1'
(rc=0)
[ovs1]$ ovs-ofctl add-flow main-net 'priority=10,in_port=eth1,actions=output:eth0'
(rc=0)
[ovs1]$ ovs-ofctl dump-flows main-net
NXST_FLOW reply (xid=0x4):
 cookie=0x0, duration=0.132s, table=0, n_packets=0, n_bytes=0, idle_age=0, priority=100,arp actions=FLOOD
 cookie=0x0, duration=0.089s, table=0, n_packets=0, n_bytes=0, idle_age=0, priority=10,in_port=1 actions=output:2
 cookie=0x0, duration=0.042s, table=0, n_packets=0, n_bytes=0, idle_age=0, priority=10,in_port=2 actions=output:1
(rc=0)
[pc]$ ping -c 3 -W 1 10.17.0.100
PING 10.17.0.100 (10.17.0.100): 56 data bytes
64 bytes from 10.17.0.100: seq=0 ttl=64 time=0.596 ms
64 bytes from 10.17.0.100: seq=1 ttl=64 time=0.092 ms
64 bytes from 10.17.0.100: seq=2 ttl=64 time=0.114 ms

--- 10.17.0.100 ping statistics ---
3 packets transmitted, 3 packets received, 0% packet loss
round-trip min/avg/max = 0.092/0.267/0.596 ms
(rc=0)
[pc]$ wget -qO- -T 3 http://10.17.0.100/
SERVER OK
(rc=0)
[srv]$ ping -c 3 -W 1 10.17.0.11
PING 10.17.0.11 (10.17.0.11): 56 data bytes
64 bytes from 10.17.0.11: seq=0 ttl=64 time=0.072 ms
64 bytes from 10.17.0.11: seq=1 ttl=64 time=0.086 ms
64 bytes from 10.17.0.11: seq=2 ttl=64 time=0.095 ms

--- 10.17.0.11 ping statistics ---
3 packets transmitted, 3 packets received, 0% packet loss
round-trip min/avg/max = 0.072/0.084/0.095 ms
(rc=0)
[srv]$ wget -qO- -T 3 http://10.17.0.11/
INSIDE OK
(rc=0)

## Korak 5 – Pokušaj bez stanja (stateless)
[ovs1]$ ovs-ofctl del-flows main-net 'in_port=eth1'
(rc=0)
[ovs1]$ ovs-ofctl add-flow main-net 'priority=10,in_port=eth1,actions=drop'
(rc=0)
[ovs1]$ ovs-ofctl dump-flows main-net
NXST_FLOW reply (xid=0x4):
 cookie=0x0, duration=4.469s, table=0, n_packets=4, n_bytes=168, idle_age=2, priority=100,arp actions=FLOOD
 cookie=0x0, duration=4.426s, table=0, n_packets=18, n_bytes=1532, idle_age=0, priority=10,in_port=1 actions=output:2
 cookie=0x0, duration=0.040s, table=0, n_packets=0, n_bytes=0, idle_age=0, priority=10,in_port=2 actions=drop
(rc=0)
[pc]$ ping -c 3 -W 1 10.17.0.100
PING 10.17.0.100 (10.17.0.100): 56 data bytes

--- 10.17.0.100 ping statistics ---
3 packets transmitted, 0 packets received, 100% packet loss
(rc=1)
[pc]$ wget -qO- -T 3 http://10.17.0.100/
wget: download timed out
(rc=1)
[ovs1]$ ovs-ofctl dump-flows main-net
NXST_FLOW reply (xid=0x4):
 cookie=0x0, duration=10.616s, table=0, n_packets=10, n_bytes=420, idle_age=0, priority=100,arp actions=FLOOD
 cookie=0x0, duration=10.573s, table=0, n_packets=24, n_bytes=2048, idle_age=0, priority=10,in_port=1 actions=output:2
 cookie=0x0, duration=6.187s, table=0, n_packets=7, n_bytes=590, idle_age=0, priority=10,in_port=2 actions=drop
(rc=0)

## Korak 6 – Praćenje konekcija: tabela 0 i tabela 1
[ovs1]$ ovs-ofctl del-flows main-net
(rc=0)
[ovs1]$ ovs-ofctl add-flow main-net 'table=0,priority=100,arp,actions=flood'
(rc=0)
[ovs1]$ ovs-ofctl add-flow main-net 'table=0,priority=50,ip,actions=ct(table=1)'
(rc=0)
[ovs1]$ ovs-ofctl add-flow main-net 'table=0,priority=0,actions=drop'
(rc=0)
[ovs1]$ ovs-ofctl add-flow main-net 'table=1,priority=200,ct_state=+inv+trk,ip,actions=drop'
(rc=0)
[ovs1]$ ovs-ofctl add-flow main-net 'table=1,priority=100,ct_state=+new+trk,in_port=eth0,ip,actions=ct(commit),output:eth1'
(rc=0)
[ovs1]$ ovs-ofctl add-flow main-net 'table=1,priority=100,ct_state=+est+trk,in_port=eth0,ip,actions=output:eth1'
(rc=0)
[ovs1]$ ovs-ofctl add-flow main-net 'table=1,priority=100,ct_state=+est+trk,in_port=eth1,ip,actions=output:eth0'
(rc=0)
[ovs1]$ ovs-ofctl add-flow main-net 'table=1,priority=90,ct_state=+new+trk,in_port=eth1,ip,actions=drop'
(rc=0)
[ovs1]$ ovs-ofctl add-flow main-net 'table=1,priority=0,actions=drop'
(rc=0)
[ovs1]$ ovs-ofctl dump-flows main-net
NXST_FLOW reply (xid=0x4):
 cookie=0x0, duration=0.362s, table=0, n_packets=0, n_bytes=0, idle_age=0, priority=100,arp actions=FLOOD
 cookie=0x0, duration=0.320s, table=0, n_packets=0, n_bytes=0, idle_age=0, priority=50,ip actions=ct(table=1)
 cookie=0x0, duration=0.284s, table=0, n_packets=0, n_bytes=0, idle_age=0, priority=0 actions=drop
 cookie=0x0, duration=0.244s, table=1, n_packets=0, n_bytes=0, idle_age=0, priority=200,ct_state=+inv+trk,ip actions=drop
 cookie=0x0, duration=0.206s, table=1, n_packets=0, n_bytes=0, idle_age=0, priority=100,ct_state=+new+trk,ip,in_port=1 actions=ct(commit),output:2
 cookie=0x0, duration=0.081s, table=1, n_packets=0, n_bytes=0, idle_age=0, priority=90,ct_state=+new+trk,ip,in_port=2 actions=drop
 cookie=0x0, duration=0.168s, table=1, n_packets=0, n_bytes=0, idle_age=0, priority=100,ct_state=+est+trk,ip,in_port=1 actions=output:2
 cookie=0x0, duration=0.124s, table=1, n_packets=0, n_bytes=0, idle_age=0, priority=100,ct_state=+est+trk,ip,in_port=2 actions=output:1
 cookie=0x0, duration=0.035s, table=1, n_packets=0, n_bytes=0, idle_age=0, priority=0 actions=drop
(rc=0)

## Korak 7 – Veze iz unutrašnje mreže prolaze
[pc]$ ping -c 3 -W 1 10.17.0.100
PING 10.17.0.100 (10.17.0.100): 56 data bytes
64 bytes from 10.17.0.100: seq=0 ttl=64 time=0.562 ms
64 bytes from 10.17.0.100: seq=1 ttl=64 time=0.236 ms
64 bytes from 10.17.0.100: seq=2 ttl=64 time=0.120 ms

--- 10.17.0.100 ping statistics ---
3 packets transmitted, 3 packets received, 0% packet loss
round-trip min/avg/max = 0.120/0.306/0.562 ms
(rc=0)
[pc]$ wget -qO- -T 3 http://10.17.0.100/
SERVER OK
(rc=0)
[pc]$ wget -qO- -T 3 http://10.17.0.100:8080/
SERVER OK
(rc=0)

## Korak 8 – Veze iz spoljašnje mreže se blokiraju
[srv]$ ping -c 3 -W 1 10.17.0.11
PING 10.17.0.11 (10.17.0.11): 56 data bytes

--- 10.17.0.11 ping statistics ---
3 packets transmitted, 0 packets received, 100% packet loss
(rc=1)
[srv]$ wget -qO- -T 3 http://10.17.0.11/
wget: download timed out
(rc=1)

## Korak 9 – Brojači pravila
[ovs1]$ ovs-ofctl dump-flows main-net
NXST_FLOW reply (xid=0x4):
 cookie=0x0, duration=8.673s, table=0, n_packets=0, n_bytes=0, idle_age=8, priority=100,arp actions=FLOOD
 cookie=0x0, duration=8.631s, table=0, n_packets=38, n_bytes=3145, idle_age=1, priority=50,ip actions=ct(table=1)
 cookie=0x0, duration=8.595s, table=0, n_packets=1, n_bytes=70, idle_age=8, priority=0 actions=drop
 cookie=0x0, duration=8.555s, table=1, n_packets=2, n_bytes=148, idle_age=3, priority=200,ct_state=+inv+trk,ip actions=drop
 cookie=0x0, duration=8.517s, table=1, n_packets=3, n_bytes=246, idle_age=6, priority=100,ct_state=+new+trk,ip,in_port=1 actions=ct(commit),output:2
 cookie=0x0, duration=8.392s, table=1, n_packets=6, n_bytes=516, idle_age=1, priority=90,ct_state=+new+trk,ip,in_port=2 actions=drop
 cookie=0x0, duration=8.479s, table=1, n_packets=14, n_bytes=1167, idle_age=6, priority=100,ct_state=+est+trk,ip,in_port=1 actions=output:2
 cookie=0x0, duration=8.435s, table=1, n_packets=13, n_bytes=1068, idle_age=6, priority=100,ct_state=+est+trk,ip,in_port=2 actions=output:1
 cookie=0x0, duration=8.346s, table=1, n_packets=0, n_bytes=0, idle_age=8, priority=0 actions=drop
(rc=0)

## Korak 10 – Samo ICMP i TCP 80
[ovs1]$ ovs-ofctl del-flows main-net 'table=1,ct_state=+new+trk,in_port=eth0,ip'
(rc=0)
[ovs1]$ ovs-ofctl add-flow main-net 'table=1,priority=100,ct_state=+new+trk,in_port=eth0,icmp,actions=ct(commit),output:eth1'
(rc=0)
[ovs1]$ ovs-ofctl add-flow main-net 'table=1,priority=100,ct_state=+new+trk,in_port=eth0,tcp,tp_dst=80,actions=ct(commit),output:eth1'
(rc=0)
[ovs1]$ ovs-ofctl dump-flows main-net table=1
NXST_FLOW reply (xid=0x8):
 cookie=0x0, duration=8.728s, table=1, n_packets=2, n_bytes=148, idle_age=3, priority=200,ct_state=+inv+trk,ip actions=drop
 cookie=0x0, duration=8.652s, table=1, n_packets=14, n_bytes=1167, idle_age=6, priority=100,ct_state=+est+trk,ip,in_port=1 actions=output:2
 cookie=0x0, duration=8.608s, table=1, n_packets=13, n_bytes=1068, idle_age=6, priority=100,ct_state=+est+trk,ip,in_port=2 actions=output:1
 cookie=0x0, duration=0.092s, table=1, n_packets=0, n_bytes=0, idle_age=0, priority=100,ct_state=+new+trk,icmp,in_port=1 actions=ct(commit),output:2
 cookie=0x0, duration=0.048s, table=1, n_packets=0, n_bytes=0, idle_age=0, priority=100,ct_state=+new+trk,tcp,in_port=1,tp_dst=80 actions=ct(commit),output:2
 cookie=0x0, duration=8.565s, table=1, n_packets=6, n_bytes=516, idle_age=1, priority=90,ct_state=+new+trk,ip,in_port=2 actions=drop
 cookie=0x0, duration=8.519s, table=1, n_packets=0, n_bytes=0, idle_age=8, priority=0 actions=drop
(rc=0)
[pc]$ ping -c 3 -W 1 10.17.0.100
PING 10.17.0.100 (10.17.0.100): 56 data bytes
64 bytes from 10.17.0.100: seq=0 ttl=64 time=0.300 ms
64 bytes from 10.17.0.100: seq=1 ttl=64 time=0.103 ms
64 bytes from 10.17.0.100: seq=2 ttl=64 time=0.131 ms

--- 10.17.0.100 ping statistics ---
3 packets transmitted, 3 packets received, 0% packet loss
round-trip min/avg/max = 0.103/0.178/0.300 ms
(rc=0)
[pc]$ wget -qO- -T 3 http://10.17.0.100/
SERVER OK
(rc=0)
[pc]$ wget -qO- -T 3 http://10.17.0.100:8080/
wget: download timed out
(rc=1)
[srv]$ ping -c 3 -W 1 10.17.0.11
PING 10.17.0.11 (10.17.0.11): 56 data bytes

--- 10.17.0.11 ping statistics ---
3 packets transmitted, 0 packets received, 100% packet loss
(rc=1)
[srv]$ wget -qO- -T 3 http://10.17.0.11/
wget: download timed out
(rc=1)
[ovs1]$ ovs-ofctl dump-flows main-net table=1
NXST_FLOW reply (xid=0x8):
 cookie=0x0, duration=20.047s, table=1, n_packets=3, n_bytes=222, idle_age=6, priority=200,ct_state=+inv+trk,ip actions=drop
 cookie=0x0, duration=19.971s, table=1, n_packets=22, n_bytes=1846, idle_age=9, priority=100,ct_state=+est+trk,ip,in_port=1 actions=output:2
 cookie=0x0, duration=19.927s, table=1, n_packets=21, n_bytes=1749, idle_age=9, priority=100,ct_state=+est+trk,ip,in_port=2 actions=output:1
 cookie=0x0, duration=11.411s, table=1, n_packets=1, n_bytes=98, idle_age=11, priority=100,ct_state=+new+trk,icmp,in_port=1 actions=ct(commit),output:2
 cookie=0x0, duration=11.367s, table=1, n_packets=1, n_bytes=74, idle_age=9, priority=100,ct_state=+new+trk,tcp,in_port=1,tp_dst=80 actions=ct(commit),output:2
 cookie=0x0, duration=19.884s, table=1, n_packets=12, n_bytes=1032, idle_age=0, priority=90,ct_state=+new+trk,ip,in_port=2 actions=drop
 cookie=0x0, duration=19.838s, table=1, n_packets=3, n_bytes=222, idle_age=7, priority=0 actions=drop
(rc=0)
```
## Kompletan izlaz izvrsavanja (runA.txt)
```

## Korak 1 – Topologija

## Korak 2 – IP adrese i HTTP serveri
[pc]$ ip link set eth0 up
(rc=0)
[pc]$ ip addr add 10.17.0.11/24 dev eth0
(rc=0)
[pc]$ echo '#!/bin/sh' > /srv.sh
(rc=0)
[pc]$ echo 'while read l; do [ "${#l}" -le 1 ] && break; done' >> /srv.sh
(rc=0)
[pc]$ echo 'printf "HTTP/1.0 200 OK\r\nContent-Length: 10\r\n\r\nINSIDE OK\n"' >> /srv.sh
(rc=0)
[pc]$ chmod +x /srv.sh
(rc=0)
[pc]$ nc -lk -p 80 -e /srv.sh &
[srv]$ ip link set eth0 up
(rc=0)
[srv]$ ip addr add 10.17.0.100/24 dev eth0
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
[srv]$ nc -lk -p 8080 -e /srv.sh &

## Korak 3 – Bridge main-net bez pravila
[ovs1]$ ovs-vsctl del-port br0 eth0
(rc=0)
[ovs1]$ ovs-vsctl del-port br0 eth1
(rc=0)
[ovs1]$ ovs-vsctl add-br main-net
(rc=0)
[ovs1]$ ovs-vsctl add-port main-net eth0
(rc=0)
[ovs1]$ ovs-vsctl add-port main-net eth1
(rc=0)
[ovs1]$ ovs-ofctl del-flows main-net
(rc=0)
[ovs1]$ ovs-ofctl dump-flows main-net
NXST_FLOW reply (xid=0x4):
(rc=0)
[pc]$ ping -c 3 -W 1 10.17.0.100
PING 10.17.0.100 (10.17.0.100): 56 data bytes

--- 10.17.0.100 ping statistics ---
3 packets transmitted, 0 packets received, 100% packet loss
(rc=1)

## Korak 4 – Prosleđivanje bez zaštite
[ovs1]$ ovs-ofctl add-flow main-net 'priority=100,arp,actions=flood'
(rc=0)
[ovs1]$ ovs-ofctl add-flow main-net 'priority=10,in_port=eth0,actions=output:eth1'
(rc=0)
[ovs1]$ ovs-ofctl add-flow main-net 'priority=10,in_port=eth1,actions=output:eth0'
(rc=0)
[ovs1]$ ovs-ofctl dump-flows main-net
NXST_FLOW reply (xid=0x4):
 cookie=0x0, duration=0.115s, table=0, n_packets=0, n_bytes=0, idle_age=0, priority=100,arp actions=FLOOD
 cookie=0x0, duration=0.079s, table=0, n_packets=0, n_bytes=0, idle_age=0, priority=10,in_port=1 actions=output:2
 cookie=0x0, duration=0.037s, table=0, n_packets=0, n_bytes=0, idle_age=0, priority=10,in_port=2 actions=output:1
(rc=0)
[pc]$ ping -c 3 -W 1 10.17.0.100
PING 10.17.0.100 (10.17.0.100): 56 data bytes
64 bytes from 10.17.0.100: seq=0 ttl=64 time=0.946 ms
64 bytes from 10.17.0.100: seq=1 ttl=64 time=0.069 ms
64 bytes from 10.17.0.100: seq=2 ttl=64 time=0.082 ms

--- 10.17.0.100 ping statistics ---
3 packets transmitted, 3 packets received, 0% packet loss
round-trip min/avg/max = 0.069/0.365/0.946 ms
(rc=0)
[pc]$ wget -qO- -T 3 http://10.17.0.100/
SERVER OK
(rc=0)
[srv]$ ping -c 3 -W 1 10.17.0.11
PING 10.17.0.11 (10.17.0.11): 56 data bytes
64 bytes from 10.17.0.11: seq=0 ttl=64 time=0.110 ms
64 bytes from 10.17.0.11: seq=1 ttl=64 time=0.080 ms
64 bytes from 10.17.0.11: seq=2 ttl=64 time=0.095 ms

--- 10.17.0.11 ping statistics ---
3 packets transmitted, 3 packets received, 0% packet loss
round-trip min/avg/max = 0.080/0.095/0.110 ms
(rc=0)
[srv]$ wget -qO- -T 3 http://10.17.0.11/
INSIDE OK
(rc=0)

## Korak 5 – Pokušaj bez stanja (stateless)
[ovs1]$ ovs-ofctl del-flows main-net 'in_port=eth1'
(rc=0)
[ovs1]$ ovs-ofctl add-flow main-net 'priority=10,in_port=eth1,actions=drop'
(rc=0)
[ovs1]$ ovs-ofctl dump-flows main-net
NXST_FLOW reply (xid=0x4):
 cookie=0x0, duration=4.449s, table=0, n_packets=2, n_bytes=84, idle_age=4, priority=100,arp actions=FLOOD
 cookie=0x0, duration=4.413s, table=0, n_packets=19, n_bytes=1602, idle_age=0, priority=10,in_port=1 actions=output:2
 cookie=0x0, duration=0.038s, table=0, n_packets=0, n_bytes=0, idle_age=0, priority=10,in_port=2 actions=drop
(rc=0)
[pc]$ ping -c 3 -W 1 10.17.0.100
PING 10.17.0.100 (10.17.0.100): 56 data bytes

--- 10.17.0.100 ping statistics ---
3 packets transmitted, 0 packets received, 100% packet loss
(rc=1)
[pc]$ wget -qO- -T 3 http://10.17.0.100/
wget: download timed out
(rc=1)
[ovs1]$ ovs-ofctl dump-flows main-net
NXST_FLOW reply (xid=0x4):
 cookie=0x0, duration=10.605s, table=0, n_packets=5, n_bytes=210, idle_age=0, priority=100,arp actions=FLOOD
 cookie=0x0, duration=10.569s, table=0, n_packets=25, n_bytes=2118, idle_age=0, priority=10,in_port=1 actions=output:2
 cookie=0x0, duration=6.194s, table=0, n_packets=7, n_bytes=590, idle_age=0, priority=10,in_port=2 actions=drop
(rc=0)

## Korak 6 – Praćenje konekcija: tabela 0 i tabela 1
[ovs1]$ ovs-ofctl del-flows main-net
(rc=0)
[ovs1]$ ovs-ofctl add-flow main-net 'table=0,priority=100,arp,actions=flood'
(rc=0)
[ovs1]$ ovs-ofctl add-flow main-net 'table=0,priority=50,ip,actions=ct(table=1)'
(rc=0)
[ovs1]$ ovs-ofctl add-flow main-net 'table=0,priority=0,actions=drop'
(rc=0)
[ovs1]$ ovs-ofctl add-flow main-net 'table=1,priority=200,ct_state=+inv+trk,ip,actions=drop'
(rc=0)
[ovs1]$ ovs-ofctl add-flow main-net 'table=1,priority=100,ct_state=+new+trk,in_port=eth0,ip,actions=ct(commit),output:eth1'
(rc=0)
[ovs1]$ ovs-ofctl add-flow main-net 'table=1,priority=100,ct_state=+est+trk,in_port=eth0,ip,actions=output:eth1'
(rc=0)
[ovs1]$ ovs-ofctl add-flow main-net 'table=1,priority=100,ct_state=+est+trk,in_port=eth1,ip,actions=output:eth0'
(rc=0)
[ovs1]$ ovs-ofctl add-flow main-net 'table=1,priority=90,ct_state=+new+trk,in_port=eth1,ip,actions=drop'
(rc=0)
[ovs1]$ ovs-ofctl add-flow main-net 'table=1,priority=0,actions=drop'
(rc=0)
[ovs1]$ ovs-ofctl dump-flows main-net
NXST_FLOW reply (xid=0x4):
 cookie=0x0, duration=0.372s, table=0, n_packets=0, n_bytes=0, idle_age=0, priority=100,arp actions=FLOOD
 cookie=0x0, duration=0.332s, table=0, n_packets=0, n_bytes=0, idle_age=0, priority=50,ip actions=ct(table=1)
 cookie=0x0, duration=0.291s, table=0, n_packets=0, n_bytes=0, idle_age=0, priority=0 actions=drop
 cookie=0x0, duration=0.246s, table=1, n_packets=0, n_bytes=0, idle_age=0, priority=200,ct_state=+inv+trk,ip actions=drop
 cookie=0x0, duration=0.207s, table=1, n_packets=0, n_bytes=0, idle_age=0, priority=100,ct_state=+new+trk,ip,in_port=1 actions=ct(commit),output:2
 cookie=0x0, duration=0.082s, table=1, n_packets=0, n_bytes=0, idle_age=0, priority=90,ct_state=+new+trk,ip,in_port=2 actions=drop
 cookie=0x0, duration=0.162s, table=1, n_packets=0, n_bytes=0, idle_age=0, priority=100,ct_state=+est+trk,ip,in_port=1 actions=output:2
 cookie=0x0, duration=0.123s, table=1, n_packets=0, n_bytes=0, idle_age=0, priority=100,ct_state=+est+trk,ip,in_port=2 actions=output:1
 cookie=0x0, duration=0.042s, table=1, n_packets=0, n_bytes=0, idle_age=0, priority=0 actions=drop
(rc=0)

## Korak 7 – Veze iz unutrašnje mreže prolaze
[pc]$ ping -c 3 -W 1 10.17.0.100
PING 10.17.0.100 (10.17.0.100): 56 data bytes
64 bytes from 10.17.0.100: seq=0 ttl=64 time=0.367 ms
64 bytes from 10.17.0.100: seq=1 ttl=64 time=0.252 ms
64 bytes from 10.17.0.100: seq=2 ttl=64 time=0.088 ms

--- 10.17.0.100 ping statistics ---
3 packets transmitted, 3 packets received, 0% packet loss
round-trip min/avg/max = 0.088/0.235/0.367 ms
(rc=0)
[pc]$ wget -qO- -T 3 http://10.17.0.100/
SERVER OK
(rc=0)
[pc]$ wget -qO- -T 3 http://10.17.0.100:8080/
SERVER OK
(rc=0)

## Korak 8 – Veze iz spoljašnje mreže se blokiraju
[srv]$ ping -c 3 -W 1 10.17.0.11
PING 10.17.0.11 (10.17.0.11): 56 data bytes

--- 10.17.0.11 ping statistics ---
3 packets transmitted, 0 packets received, 100% packet loss
(rc=1)
[srv]$ wget -qO- -T 3 http://10.17.0.11/
wget: download timed out
(rc=1)

## Korak 9 – Brojači pravila
[ovs1]$ ovs-ofctl dump-flows main-net
NXST_FLOW reply (xid=0x4):
 cookie=0x0, duration=8.687s, table=0, n_packets=0, n_bytes=0, idle_age=8, priority=100,arp actions=FLOOD
 cookie=0x0, duration=8.647s, table=0, n_packets=38, n_bytes=3145, idle_age=0, priority=50,ip actions=ct(table=1)
 cookie=0x0, duration=8.606s, table=0, n_packets=0, n_bytes=0, idle_age=8, priority=0 actions=drop
 cookie=0x0, duration=8.561s, table=1, n_packets=2, n_bytes=148, idle_age=3, priority=200,ct_state=+inv+trk,ip actions=drop
 cookie=0x0, duration=8.523s, table=1, n_packets=3, n_bytes=246, idle_age=6, priority=100,ct_state=+new+trk,ip,in_port=1 actions=ct(commit),output:2
 cookie=0x0, duration=8.398s, table=1, n_packets=6, n_bytes=516, idle_age=0, priority=90,ct_state=+new+trk,ip,in_port=2 actions=drop
 cookie=0x0, duration=8.478s, table=1, n_packets=14, n_bytes=1167, idle_age=6, priority=100,ct_state=+est+trk,ip,in_port=1 actions=output:2
 cookie=0x0, duration=8.439s, table=1, n_packets=13, n_bytes=1068, idle_age=6, priority=100,ct_state=+est+trk,ip,in_port=2 actions=output:1
 cookie=0x0, duration=8.358s, table=1, n_packets=0, n_bytes=0, idle_age=8, priority=0 actions=drop
(rc=0)

## Korak 10 – Samo ICMP i TCP 80
[ovs1]$ ovs-ofctl del-flows main-net 'table=1,ct_state=+new+trk,in_port=eth0,ip'
(rc=0)
[ovs1]$ ovs-ofctl add-flow main-net 'table=1,priority=100,ct_state=+new+trk,in_port=eth0,icmp,actions=ct(commit),output:eth1'
(rc=0)
[ovs1]$ ovs-ofctl add-flow main-net 'table=1,priority=100,ct_state=+new+trk,in_port=eth0,tcp,tp_dst=80,actions=ct(commit),output:eth1'
(rc=0)
[ovs1]$ ovs-ofctl dump-flows main-net table=1
NXST_FLOW reply (xid=0x8):
 cookie=0x0, duration=8.744s, table=1, n_packets=2, n_bytes=148, idle_age=3, priority=200,ct_state=+inv+trk,ip actions=drop
 cookie=0x0, duration=8.660s, table=1, n_packets=14, n_bytes=1167, idle_age=6, priority=100,ct_state=+est+trk,ip,in_port=1 actions=output:2
 cookie=0x0, duration=8.621s, table=1, n_packets=13, n_bytes=1068, idle_age=6, priority=100,ct_state=+est+trk,ip,in_port=2 actions=output:1
 cookie=0x0, duration=0.089s, table=1, n_packets=0, n_bytes=0, idle_age=0, priority=100,ct_state=+new+trk,icmp,in_port=1 actions=ct(commit),output:2
 cookie=0x0, duration=0.039s, table=1, n_packets=0, n_bytes=0, idle_age=0, priority=100,ct_state=+new+trk,tcp,in_port=1,tp_dst=80 actions=ct(commit),output:2
 cookie=0x0, duration=8.580s, table=1, n_packets=6, n_bytes=516, idle_age=1, priority=90,ct_state=+new+trk,ip,in_port=2 actions=drop
 cookie=0x0, duration=8.540s, table=1, n_packets=0, n_bytes=0, idle_age=8, priority=0 actions=drop
(rc=0)
[pc]$ ping -c 3 -W 1 10.17.0.100
PING 10.17.0.100 (10.17.0.100): 56 data bytes
64 bytes from 10.17.0.100: seq=0 ttl=64 time=0.621 ms
64 bytes from 10.17.0.100: seq=1 ttl=64 time=0.086 ms
64 bytes from 10.17.0.100: seq=2 ttl=64 time=0.101 ms

--- 10.17.0.100 ping statistics ---
3 packets transmitted, 3 packets received, 0% packet loss
round-trip min/avg/max = 0.086/0.269/0.621 ms
(rc=0)
[pc]$ wget -qO- -T 3 http://10.17.0.100/
SERVER OK
(rc=0)
[pc]$ wget -qO- -T 3 http://10.17.0.100:8080/
wget: download timed out
(rc=1)
[srv]$ ping -c 3 -W 1 10.17.0.11
PING 10.17.0.11 (10.17.0.11): 56 data bytes

--- 10.17.0.11 ping statistics ---
3 packets transmitted, 0 packets received, 100% packet loss
(rc=1)
[srv]$ wget -qO- -T 3 http://10.17.0.11/
wget: download timed out
(rc=1)
[ovs1]$ ovs-ofctl dump-flows main-net table=1
NXST_FLOW reply (xid=0x8):
 cookie=0x0, duration=20.047s, table=1, n_packets=3, n_bytes=222, idle_age=6, priority=200,ct_state=+inv+trk,ip actions=drop
 cookie=0x0, duration=19.963s, table=1, n_packets=22, n_bytes=1846, idle_age=9, priority=100,ct_state=+est+trk,ip,in_port=1 actions=output:2
 cookie=0x0, duration=19.924s, table=1, n_packets=22, n_bytes=1815, idle_age=9, priority=100,ct_state=+est+trk,ip,in_port=2 actions=output:1
 cookie=0x0, duration=11.392s, table=1, n_packets=1, n_bytes=98, idle_age=11, priority=100,ct_state=+new+trk,icmp,in_port=1 actions=ct(commit),output:2
 cookie=0x0, duration=11.342s, table=1, n_packets=1, n_bytes=74, idle_age=9, priority=100,ct_state=+new+trk,tcp,in_port=1,tp_dst=80 actions=ct(commit),output:2
 cookie=0x0, duration=19.883s, table=1, n_packets=12, n_bytes=1032, idle_age=0, priority=90,ct_state=+new+trk,ip,in_port=2 actions=drop
 cookie=0x0, duration=19.843s, table=1, n_packets=3, n_bytes=222, idle_age=7, priority=0 actions=drop
(rc=0)
```
