# Vežba 9 – provera u pravom simulatoru

Topologija iz zadatka napravljena je u simulatoru (API + veb UI, port 3098, `gns3/openvswitch`, `jasonrivers/nagios`,
`vezba9-linux`, VPCS, MikroTik CHR 7.23.8 u QEMU/KVM, `docker-sonic-vs`), komande iz rešenja su izvršene redom kroz
konzole simulatora (WebSocket), a Nagios je otvaran kroz HTTP konzolu simulatora (`/proxy/<token>/nagios/`).

| Slika | Šta pokazuje |
|---|---|
| 01 | Topologija i svi čvorovi pokrenuti |
| 02, 03 | Nagios: 7/7 hostova UP; HTTP i SNMP uptime OK na SERVER1/SERVER2 i na R1 |
| 04 | SERVER1: httpd + snmpd, `snmpget` sa zajednicom lab9 |
| 05 | VPC1: ping SERVER1 i R1 kroz SW1 |
| 06 | R1 (RouterOS): adresa, SNMP, `nadzor` syslog akcija ka 10.9.0.20 |
| 07 | SYSLOG: fajlovi po hostu (uključujući 10.9.0.1.log od R1), poruke Nagios-a |
| 08, 09 | Korak 7: SERVER2 DOWN, HTTP i SNMP uptime CRITICAL |
| 10 | Obaveštenja: `dezurni` / `notify-host-by-syslog` |

Rezultat: svi koraci 2–7 prošli. SERVER2 prelazi u HARD DOWN za ~97 s.

SONiC u ovom prolazu bio je `docker-sonic-vs`; zato je naknadno ista SONiC komanda proverena na pravom SONiC-VS VM-u
(vidi Vežbu 8). Nalaz: `config interface ip add eth0 10.9.0.2/24` bez gateway-a upiše adresu u ConfigDB, ali je
ne primeni na eth0; sa `... 10.9.0.2/24 10.9.0.1` adresa se primenjuje (`show management_interface address`).
Zadatak i rešenje su ispravljeni. `config syslog add` i `config snmp community add lab9 RO` rade na VM-u.
`localhost` HTTP WARNING (401) potiče iz podrazumevane konfiguracije Nagios slike.
