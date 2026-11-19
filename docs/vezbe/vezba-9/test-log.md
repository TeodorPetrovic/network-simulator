# Vežba 9 – test log (za održavanje, nije deo vežbe)

**Važno: R1 (MikroTik) i SONiC nisu testirani.** Ti uređaji nisu dostupni na test mašini: njihove komande
(blokovi `data-dev="mikrotik"` i `"sonic"` u Koracima 2, 5 i 6) **nisu izvršene ni proverene**. Njihovi Nagios
hostovi (`R1`, `SONiC`) i servis `SNMP uptime` na njima su definisani i učitani (konfiguracija prolazi `nagios -v`),
ali su u testu DOWN/CRITICAL jer uređaja nema u mreži. Sve ostalo (NAGIOS, SYSLOG, SERVER1, SERVER2, VPC1, SW1)
testirano je na pravim kontejnerima. Veb interfejs je proveravan preko CGI-ja (`statusjson.cgi`, `objectjson.cgi`)
iz samog NAGIOS kontejnera, ne kroz HTTP konzolu simulatora (simulator nije pokretan).

Uređaji: `jasonrivers/nagios:latest` (Nagios Core 4.5.12, nagios-plugins 2.5), `vezba9-linux` (alpine 3.21 +
net-snmp 5.9.4, rsyslog 8.2410.0, util-linux logger, busybox-extras httpd), `gns3/openvswitch` (SW1, svi portovi u
`br0`, NORMAL), pravi `vpcs` + `ubridge` (VPC1). Detalji harness-a: `src/README.md` i docstring `src/run-test.py`.

Napomene iz testa:
- U pomoćnoj (aux) konzoli simulatora (`/gns3/bin/busybox sh`) BusyBox koristi svoje aplete umesto programa slike
  (npr. `logger` bez opcije `-n`). Zato se `logger` u vežbi kuca na SERVER1 (glavna konzola, `/bin/sh` slike), a
  Nagios komande za obaveštenja koriste apsolutnu putanju `/usr/bin/logger`.
- `snmp.conf` slike NAGIOS ima `mibs +ALL`, pa `snmpget`/`snmpwalk` bez `-m` ispisuju desetine upozorenja o MIB
  fajlovima koji nedostaju; sa `-m SNMPv2-MIB` / `-m IF-MIB` izlaz je čist (proverava se u testu).
- Userspace OVS (netdev) u Docker testu ne dopunjuje delimične TCP/UDP checksume sa veth-a: bez
  `ethtool -K eth0 tx off` na krajnjim kontejnerima radi samo ping (HTTP i SNMP ne). Ovo je samo podešavanje testa.
- Nagios prihvata `\` na kraju reda kao nastavak direktive (komande `notify-*-by-syslog`), provereno.
- Vremena u testu: svi hostovi UP ~60 s posle reload-a; SERVER2 HARD DOWN 96–111 s posle zaustavljanja.

## Rezultat

Tri uzastopna pokretanja od čistog stanja nad konačnim fragmentima, svako 32/32 PASS:

| run | hostovi UP (k3) | servisi OK (k4) | SERVER2 HARD DOWN (k7) | rezultat |
|-----|-----------------|-----------------|------------------------|----------|
| 1   | 61 s            | 61 s            | 96 s                   | 32/32 PASS |
| 2   | 66 s            | 56 s            | 101 s                  | 32/32 PASS |
| 3   | 66 s            | 35 s            | 101 s                  | 32/32 PASS |

## Pun izlaz (run 3)

```
CHECK PASS: K1 vezba9-linux image builds from Dockerfile in task

##### Korak 2 – IP adrese, veb server i SNMP agent
[network config]
auto eth0
iface eth0 inet static
    address 10.9.0.10
    netmask 255.255.255.0
CHECK PASS: K2 nagios: ifup applied 10.9.0.10 from network config
CHECK PASS: K2 syslog: ifup applied 10.9.0.20 from network config
CHECK PASS: K2 server1: ifup applied 10.9.0.31 from network config
CHECK PASS: K2 server2: ifup applied 10.9.0.32 from network config
[vpc1]$ ip 10.9.0.50/24
Checking for duplicate address...
VPC1 : 10.9.0.50 255.255.255.0
(rc=0)
[server1]$ mkdir -p /var/www
echo "<h1>$(hostname)</h1>" > /var/www/index.html
httpd -h /var/www
cat > /etc/snmp/snmpd.conf <<'EOF'
agentAddress udp:161
rocommunity lab9 10.9.0.0/24
sysLocation Laboratorija 9
sysContact admin@lab9.local
EOF
snmpd
ps
PID   USER     TIME  COMMAND
    1 root      0:00 /bin/sh
  324 root      0:00 /bin/sh
  333 root      0:00 httpd -h /var/www
  336 root      0:00 [snmpd]
  337 root      0:00 snmpd
  338 root      0:00 ps
(rc=0)
[server2]$ mkdir -p /var/www
echo "<h1>$(hostname)</h1>" > /var/www/index.html
httpd -h /var/www
cat > /etc/snmp/snmpd.conf <<'EOF'
agentAddress udp:161
rocommunity lab9 10.9.0.0/24
sysLocation Laboratorija 9
sysContact admin@lab9.local
EOF
snmpd
ps
PID   USER     TIME  COMMAND
    1 root      0:00 /bin/sh
  325 root      0:00 /bin/sh
  334 root      0:00 httpd -h /var/www
  337 root      0:00 [snmpd]
  338 root      0:00 snmpd
  339 root      0:00 ps
(rc=0)
[mikrotik] NIJE IZVRSENO (uredjaj nije dostupan):
    /system identity set name=R1
    /ip address add address=10.9.0.1/24 interface=ether1
[sonic] NIJE IZVRSENO (uredjaj nije dostupan):
    sudo config interface ip add eth0 10.9.0.2/24
CHECK PASS: K2 VPC1 ip set
CHECK PASS: K2 server1: httpd and snmpd running
CHECK PASS: K2 server2: httpd and snmpd running
CHECK PASS: K2 HTTP from NAGIOS to SERVER2 shows its name
CHECK PASS: K2 NAGIOS pings VPC1

##### Korak 3 – hostovi, grupe hostova i servis HTTP
[nagios]$ cat > /opt/nagios/etc/conf.d/lab9.cfg <<'EOF'
define host {
    name                  lab9-host
    use                   generic-host
    check_command         check-host-alive
    check_interval        1
    retry_interval        1
    max_check_attempts    2
    notification_options  d,u,r
    contact_groups        admins
    register              0
}
define host {
    use         lab9-host
    host_name   SYSLOG
    address     10.9.0.20
}
define host {
    use         lab9-host
    host_name   SERVER1
    address     10.9.0.31
}
define host {
    use         lab9-host
    host_name   SERVER2
    address     10.9.0.32
}
define host {
    use         lab9-host
    host_name   VPC1
    address     10.9.0.50
}
define host {
    use         lab9-host
    host_name   R1
    address     10.9.0.1
}
define host {
    use         lab9-host
    host_name   SONiC
    address     10.9.0.2
}
EOF

(rc=0)
[nagios]$ cat >> /opt/nagios/etc/conf.d/lab9.cfg <<'EOF'
define hostgroup {
    hostgroup_name  serveri
    alias           Serveri
    members         SERVER1,SERVER2
}
define hostgroup {
    hostgroup_name  mrezni-uredjaji
    alias           Mrezni uredjaji
    members         R1,SONiC
}
define service {
    use                  generic-service
    hostgroup_name       serveri
    service_description  HTTP
    check_command        check_http
    check_interval       1
    retry_interval       1
    max_check_attempts   2
}
EOF
/opt/nagios/bin/nagios -v /opt/nagios/etc/nagios.cfg | tail -3
sv reload nagios
Total Errors:   0

Things look okay - No serious problems were detected during the pre-flight check
ok: run: nagios: (pid 320) 14s
(rc=0)
[wait] k3: ok after 66s
CHECK PASS: K3 wait k3
[nagios]$ S="http://nagiosadmin:nagios@localhost/nagios/cgi-bin/statusjson.cgi"
curl -s "$S?query=hostlist" | jq -c .data.hostlist
{"R1":4,"SERVER1":2,"SERVER2":2,"SONiC":4,"SYSLOG":2,"VPC1":2,"localhost":2}
(rc=0)
CHECK PASS: K3 config check: 0 errors, reload ok
CHECK PASS: K3 lab hosts UP (2)
CHECK PASS: K3 R1 and SONiC defined (absent in test -> DOWN 4 or PENDING 1)

##### Korak 4 – komanda, servis SNMP uptime i kontakt
[nagios]$ cat > /opt/nagios/etc/conf.d/nadzor.cfg <<'EOF'
define command {
    command_name  check_snmp_lab9
    command_line  $USER1$/check_snmp -H $HOSTADDRESS$ -P 2c -C $ARG1$ -o $ARG2$
}
define service {
    use                  generic-service
    hostgroup_name       serveri,mrezni-uredjaji
    service_description  SNMP uptime
    check_command        check_snmp_lab9!lab9!1.3.6.1.2.1.1.3.0
    check_interval       1
    retry_interval       1
    max_check_attempts   2
}
define command {
    command_name  notify-host-by-syslog
    command_line  /usr/bin/logger -n 10.9.0.20 -P 514 -d -t nagios \
"HOST $NOTIFICATIONTYPE$: $HOSTNAME$ je $HOSTSTATE$"
}
define command {
    command_name  notify-service-by-syslog
    command_line  /usr/bin/logger -n 10.9.0.20 -P 514 -d -t nagios \
"SERVIS $NOTIFICATIONTYPE$: $HOSTNAME$/$SERVICEDESC$ je $SERVICESTATE$"
}
define contact {
    contact_name                   dezurni
    use                            generic-contact
    alias                          Dezurni administrator
    contactgroups                  admins
    host_notification_commands     notify-host-by-syslog
    service_notification_commands  notify-service-by-syslog
}
EOF
/opt/nagios/bin/nagios -v /opt/nagios/etc/nagios.cfg | tail -3
sv reload nagios
Total Errors:   0

Things look okay - No serious problems were detected during the pre-flight check
ok: run: nagios: (pid 320) 80s
(rc=0)
[wait] k4: ok after 35s
CHECK PASS: K4 wait k4
[nagios]$ S="http://nagiosadmin:nagios@localhost/nagios/cgi-bin/statusjson.cgi"
curl -s "$S?query=servicelist&hostgroup=serveri" | jq -c .data.servicelist
{"SERVER1":{"HTTP":2,"SNMP uptime":2},"SERVER2":{"HTTP":2,"SNMP uptime":2}}
(rc=0)
CHECK PASS: K4 config check: 0 errors, reload ok
CHECK PASS: K4 HTTP and SNMP uptime OK on SERVER1 and SERVER2
CHECK PASS: K4 contact dezurni is member of admins
CHECK PASS: K4 SNMP uptime plugin output is SNMP OK - Timeticks

##### Korak 5 – syslog server
[syslog]$ mkdir -p /etc/rsyslog.d
cat > /etc/rsyslog.d/mreza.conf <<'EOF'
module(load="imudp")
template(name="poHostu" type="string" string="/var/log/mreza/%HOSTNAME%.log")
ruleset(name="mreza") {
  action(type="omfile" dynaFile="poHostu")
}
input(type="imudp" port="514" ruleset="mreza")
EOF
rsyslogd -N1
rsyslogd
netstat -lnu | grep 514
rsyslogd: version 8.2410.0, config validation run (level 1), master config /etc/rsyslog.conf
rsyslogd: End of config validation run. Bye.
udp        0      0 0.0.0.0:514             0.0.0.0:*                           
udp        0      0 :::514                  :::*
(rc=0)
[server1]$ logger -n 10.9.0.20 -P 514 -d -t proba "Poruka sa SERVER1"

(rc=0)
[syslog]$ ls /var/log/mreza
cat /var/log/mreza/SERVER1.log
SERVER1.log
2026-11-19T19:02:40.838216+00:00 SERVER1 proba Poruka sa SERVER1
(rc=0)
[mikrotik] NIJE IZVRSENO (uredjaj nije dostupan):
    /system logging action add name=nadzor target=remote remote=10.9.0.20 remote-port=514
    /system logging add topics=info action=nadzor
[sonic] NIJE IZVRSENO (uredjaj nije dostupan):
    sudo config syslog add 10.9.0.20
CHECK PASS: K5 rsyslog config valid and UDP 514 listening
CHECK PASS: K5 logger message from SERVER1 in /var/log/mreza/SERVER1.log
CHECK PASS: K5 remote message not in /var/log/messages

##### Korak 6 – SNMP upiti
[nagios]$ snmpget -v2c -c lab9 -m SNMPv2-MIB 10.9.0.31 sysName.0 sysLocation.0 sysUpTime.0
snmpwalk -v2c -c lab9 -m IF-MIB 10.9.0.32 ifDescr
snmpwalk -v2c -c lab9 -m IF-MIB 10.9.0.32 ifOperStatus
snmpget -v2c -c public -m SNMPv2-MIB 10.9.0.31 sysName.0
Created directory: /var/lib/snmp/cert_indexes
SNMPv2-MIB::sysName.0 = STRING: SERVER1
SNMPv2-MIB::sysLocation.0 = STRING: Laboratorija 9
SNMPv2-MIB::sysUpTime.0 = Timeticks: (10287) 0:01:42.87
IF-MIB::ifDescr.1 = STRING: lo
IF-MIB::ifDescr.2 = STRING: eth0
IF-MIB::ifOperStatus.1 = INTEGER: up(1)
IF-MIB::ifOperStatus.2 = INTEGER: up(1)
Timeout: No Response from 10.9.0.31.
(rc=1)
[mikrotik] NIJE IZVRSENO (uredjaj nije dostupan):
    /snmp community add name=lab9 addresses=10.9.0.0/24 read-access=yes write-access=no
    /snmp set enabled=yes location="Laboratorija 9" contact=admin@lab9.local
[sonic] NIJE IZVRSENO (uredjaj nije dostupan):
    sudo config snmp community add lab9 RO
CHECK PASS: K6 snmpget sysName/sysLocation/sysUpTime
CHECK PASS: K6 snmpwalk ifDescr/ifOperStatus eth0 up
CHECK PASS: K6 community public -> Timeout
CHECK PASS: K6 no MIB warnings with -m

##### Korak 7 – ispad servera SERVER2
[GNS3] Stop SERVER2  ->  docker stop v9_server2
[wait] k7: ok after 101s
CHECK PASS: K7 wait k7
[nagios]$ S="http://nagiosadmin:nagios@localhost/nagios/cgi-bin/statusjson.cgi"
curl -s "$S?query=host&hostname=SERVER2" | \
  jq -c '.data.host | {status, state_type, current_attempt, plugin_output}'
curl -s "$S?query=servicelist&hostname=SERVER2" | jq -c .data.servicelist
{"status":4,"state_type":1,"current_attempt":2,"plugin_output":"CRITICAL - Host Unreachable (10.9.0.32)"}
{"SERVER2":{"HTTP":16,"SNMP uptime":16}}
(rc=0)
[syslog]$ grep SERVER2 /var/log/mreza/NAGIOS.log
2026-11-19T19:04:21.342335+00:00 NAGIOS nagios HOST PROBLEM: SERVER2 je DOWN
(rc=0)
CHECK PASS: K7 SERVER2 DOWN HARD attempt 2
CHECK PASS: K7 SERVER2 HTTP and SNMP uptime CRITICAL (16)
CHECK PASS: K7 host notification on SYSLOG
CHECK PASS: K7 no service notification for SERVER2
CHECK PASS: K7 other lab hosts still UP

===== SUMMARY
PASS K1 vezba9-linux image builds from Dockerfile in task
PASS K2 nagios: ifup applied 10.9.0.10 from network config
PASS K2 syslog: ifup applied 10.9.0.20 from network config
PASS K2 server1: ifup applied 10.9.0.31 from network config
PASS K2 server2: ifup applied 10.9.0.32 from network config
PASS K2 VPC1 ip set
PASS K2 server1: httpd and snmpd running
PASS K2 server2: httpd and snmpd running
PASS K2 HTTP from NAGIOS to SERVER2 shows its name
PASS K2 NAGIOS pings VPC1
PASS K3 wait k3
PASS K3 config check: 0 errors, reload ok
PASS K3 lab hosts UP (2)
PASS K3 R1 and SONiC defined (absent in test -> DOWN 4 or PENDING 1)
PASS K4 wait k4
PASS K4 config check: 0 errors, reload ok
PASS K4 HTTP and SNMP uptime OK on SERVER1 and SERVER2
PASS K4 contact dezurni is member of admins
PASS K4 SNMP uptime plugin output is SNMP OK - Timeticks
PASS K5 rsyslog config valid and UDP 514 listening
PASS K5 logger message from SERVER1 in /var/log/mreza/SERVER1.log
PASS K5 remote message not in /var/log/messages
PASS K6 snmpget sysName/sysLocation/sysUpTime
PASS K6 snmpwalk ifDescr/ifOperStatus eth0 up
PASS K6 community public -> Timeout
PASS K6 no MIB warnings with -m
PASS K7 wait k7
PASS K7 SERVER2 DOWN HARD attempt 2
PASS K7 SERVER2 HTTP and SNMP uptime CRITICAL (16)
PASS K7 host notification on SYSLOG
PASS K7 no service notification for SERVER2
PASS K7 other lab hosts still UP
32/32 PASS
```
