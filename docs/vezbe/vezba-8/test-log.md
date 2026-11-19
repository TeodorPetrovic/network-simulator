# Vežba 8 – test log (za održavanje, nije deo vežbe)

**Šta je provereno:** sve komande iz rešenja (`<pre data-dev>` blokovi) izvršene su redom, tačno kako su zapisane:

- **R1 – pravi MikroTik CHR 7.16.2** (QEMU/KVM, serijska konzola): sve RouterOS komande iz koraka 4 i 5 i
  `/ip neighbor print` (prva prijava sa promenom lozinke uključena).
- **SW1, SW2 – docker-sonic-vs** (SONiC `master.1280150-71d15c250`, Debian 12), sa t1 početnom konfiguracijom
  kao SONiC-VS VM (10.0.0.x/31 na portovima). Ovo NIJE SONiC-VS VM iz GNS3: nema systemd/hostcfgd/docker, pa
  `config hostname` harness sam primenjuje (`hostname` + `/etc/hosts`), a `docker`/`systemctl` su zamenjeni
  skriptama (`src/sonic-stub-*`). Bez zamene, `config vlan add` i `config snmp community add` upišu konfiguraciju
  u CONFIG_DB pa padnu na `docker`/`systemctl`. Na slici fali promenljiva `BGPD_GRACEFUL_RESTART_ARG` (supervisord
  ne startuje bez nje), a lldpd posle pokretanja izgubi socket (restartuje se u `test-setup.sh`).
- **VPC1, VPC2 – pravi vpcs** iz simulatora (preko ubridge-a, kao u GNS3).
- Zamena za GNS3 NAT čvor: `v8_nat` (udhcpd 192.168.122.100-200, gateway 192.168.122.1, 8.8.8.8 na lo).
- Server za nadzor (Vežba 9) `v8_srv` 10.10.99.10 na SW2 Ethernet12: snmpget ka R1/SW1/SW2 (`lab8`), prijem
  syslog-a sa R1, `public` na R1 više ne radi.
- Kablovi su veth parovi (ne Docker mreže: Linux bridge ne propušta LLDP), sa isključenim tx checksum offload-om.
- Adresni plan: `python3 src/check-plan.py` (podmreže se ne preklapaju, gateway-i = adrese R1, DHCP opseg u VLAN-u 10,
  komande u rešenju odgovaraju tabelama iz zadatka) – sve provere prolaze.

**Šta nije provereno:** SONiC-VS kao QEMU VM u GNS3 (mapiranje adaptera eth1 → Ethernet0 je iz `lanemap.ini` iste
platforme `x86_64-kvm_x86_64-r0`/Force10-S6000), stvarni GNS3 NAT čvor, promena prompta posle `config hostname`.

Rezultat: 4 uzastopna čista pokretanja (`test-teardown.sh; test-setup.sh; run-test.py`) – 28/28 PASS.
Izlazi u rešenju su iz ovog loga (MAC adrese i vremena se menjaju od pokretanja do pokretanja).

## Pun izlaz poslednjeg pokretanja

```

##### Korak 3 – SONiC: ime, VLAN-ovi, portovi, SNMP
admin@SW1:~$ show ip interfaces
Interface    Master    IPv4 address/mask    Admin/Oper    BGP Neighbor    Neighbor IP
-----------  --------  -------------------  ------------  --------------  -------------
Ethernet0              10.0.0.0/31          up/up         ARISTA01T2      10.0.0.1
Ethernet4              10.0.0.2/31          up/up         ARISTA01T0      10.0.0.3
Ethernet8              10.0.0.4/31          up/up         ARISTA01T0      10.0.0.5
Loopback0              10.1.0.1/32          up/up         N/A             N/A
lo                     127.0.0.1/8          up/up         N/A             N/A
(rc=0)
admin@SW1:~$ sudo config hostname SW1
Please note loaded setting will be lost after system reboot. To preserve setting, run `config save`.
(rc=0)
admin@SW1:~$ sudo config interface ip remove Ethernet0 10.0.0.0/31
(rc=0)
admin@SW1:~$ sudo config interface ip remove Ethernet4 10.0.0.2/31
(rc=0)
admin@SW1:~$ sudo config interface ip remove Ethernet8 10.0.0.4/31
(rc=0)
admin@SW1:~$ sudo config vlan add 10
(rc=0)
admin@SW1:~$ sudo config vlan add 20
(rc=0)
admin@SW1:~$ sudo config vlan add 99
(rc=0)
admin@SW1:~$ sudo config vlan member add 10 Ethernet0
(rc=0)
admin@SW1:~$ sudo config vlan member add 99 Ethernet0
(rc=0)
admin@SW1:~$ sudo config vlan member add 10 Ethernet4
(rc=0)
admin@SW1:~$ sudo config vlan member add 20 Ethernet4
(rc=0)
admin@SW1:~$ sudo config vlan member add 99 Ethernet4
(rc=0)
admin@SW1:~$ sudo config vlan member add -u 10 Ethernet8
(rc=0)
admin@SW1:~$ sudo config interface ip add Vlan99 10.10.99.2/24
(rc=0)
admin@SW1:~$ sudo config snmp community add lab8 RO
SNMP community lab8 added to configuration
Restarting SNMP service...
(rc=0)
admin@SW1:~$ sudo config save -y
Running command: /usr/local/bin/sonic-cfggen -d --print-data > /etc/sonic/config_db.json
(rc=0)
admin@SW2:~$ sudo config hostname SW2
Please note loaded setting will be lost after system reboot. To preserve setting, run `config save`.
(rc=0)
admin@SW2:~$ sudo config interface ip remove Ethernet0 10.0.0.0/31
(rc=0)
admin@SW2:~$ sudo config interface ip remove Ethernet4 10.0.0.2/31
(rc=0)
admin@SW2:~$ sudo config interface ip remove Ethernet8 10.0.0.4/31
(rc=0)
admin@SW2:~$ sudo config interface ip remove Ethernet12 10.0.0.6/31
(rc=0)
admin@SW2:~$ sudo config vlan add 10
(rc=0)
admin@SW2:~$ sudo config vlan add 20
(rc=0)
admin@SW2:~$ sudo config vlan add 99
(rc=0)
admin@SW2:~$ sudo config vlan member add -u 20 Ethernet0
(rc=0)
admin@SW2:~$ sudo config vlan member add 10 Ethernet4
(rc=0)
admin@SW2:~$ sudo config vlan member add 20 Ethernet4
(rc=0)
admin@SW2:~$ sudo config vlan member add 99 Ethernet4
(rc=0)
admin@SW2:~$ sudo config vlan member add -u 20 Ethernet8
(rc=0)
admin@SW2:~$ sudo config vlan member add -u 99 Ethernet12
(rc=0)
admin@SW2:~$ sudo config interface ip add Vlan99 10.10.99.3/24
(rc=0)
admin@SW2:~$ sudo config snmp community add lab8 RO
SNMP community lab8 added to configuration
Restarting SNMP service...
(rc=0)
admin@SW2:~$ sudo config save -y
Running command: /usr/local/bin/sonic-cfggen -d --print-data > /etc/sonic/config_db.json
(rc=0)
admin@SW2:~$ show vlan brief
+-----------+---------------+------------+----------------+-------------+--------------------------+
|   VLAN ID | IP Address    | Ports      | Port Tagging   | Proxy ARP   | Static Anycast Gateway   |
+===========+===============+============+================+=============+==========================+
|        10 |               | Ethernet4  | tagged         | disabled    | disabled                 |
+-----------+---------------+------------+----------------+-------------+--------------------------+
|        20 |               | Ethernet0  | untagged       | disabled    | disabled                 |
|           |               | Ethernet4  | tagged         |             |                          |
|           |               | Ethernet8  | untagged       |             |                          |
+-----------+---------------+------------+----------------+-------------+--------------------------+
|        99 | 10.10.99.3/24 | Ethernet4  | tagged         | disabled    | disabled                 |
|           |               | Ethernet12 | untagged       |             |                          |
+-----------+---------------+------------+----------------+-------------+--------------------------+
(rc=0)
admin@SW1:~$ show vlan config
Name      VID  Member     Mode
------  -----  ---------  --------
Vlan10     10  Ethernet0  tagged
Vlan10     10  Ethernet4  tagged
Vlan10     10  Ethernet8  untagged
Vlan20     20  Ethernet4  tagged
Vlan99     99  Ethernet0  tagged
Vlan99     99  Ethernet4  tagged
(rc=0)
Name      VID  Member     Mode
------  -----  ---------  --------
Vlan10     10  Ethernet0  tagged
Vlan10     10  Ethernet4  tagged
Vlan10     10  Ethernet8  untagged
Vlan20     20  Ethernet4  tagged
Vlan99     99  Ethernet0  tagged
Vlan99     99  Ethernet4  tagged

CHECK PASS: K3 sw1 VLAN members as in table 
admin@SW1:~$ show ip interfaces
Interface    Master    IPv4 address/mask    Admin/Oper    BGP Neighbor    Neighbor IP
-----------  --------  -------------------  ------------  --------------  -------------
Loopback0              10.1.0.1/32          up/up         N/A             N/A
Vlan99                 10.10.99.2/24        up/up         N/A             N/A
lo                     127.0.0.1/8          up/up         N/A             N/A
(rc=0)
CHECK PASS: K3 sw1 Vlan99 10.10.99.2/24, no 10.0.0.x on used ports 
CHECK PASS: K3 sw1 saved config_db.json: hostname, VLANs, SNMP community 
admin@SW2:~$ show vlan config
Name      VID  Member      Mode
------  -----  ----------  --------
Vlan10     10  Ethernet4   tagged
Vlan20     20  Ethernet0   untagged
Vlan20     20  Ethernet4   tagged
Vlan20     20  Ethernet8   untagged
Vlan99     99  Ethernet4   tagged
Vlan99     99  Ethernet12  untagged
(rc=0)
Name      VID  Member      Mode
------  -----  ----------  --------
Vlan10     10  Ethernet4   tagged
Vlan20     20  Ethernet0   untagged
Vlan20     20  Ethernet4   tagged
Vlan20     20  Ethernet8   untagged
Vlan99     99  Ethernet4   tagged
Vlan99     99  Ethernet12  untagged

CHECK PASS: K3 sw2 VLAN members as in table 
admin@SW2:~$ show ip interfaces
Interface    Master    IPv4 address/mask    Admin/Oper    BGP Neighbor    Neighbor IP
-----------  --------  -------------------  ------------  --------------  -------------
Loopback0              10.1.0.1/32          up/up         N/A             N/A
Vlan99                 10.10.99.3/24        up/up         N/A             N/A
lo                     127.0.0.1/8          up/up         N/A             N/A
(rc=0)
CHECK PASS: K3 sw2 Vlan99 10.10.99.3/24, no 10.0.0.x on used ports 
CHECK PASS: K3 sw2 saved config_db.json: hostname, VLANs, SNMP community 

##### Korak 4 – R1: ime, korisnici, VLAN podinterfejsi, adrese
[R1] prijava admin (prazna lozinka) -> licenca n -> nova lozinka Vezba8admin
[admin@R1] > /system identity set name=R1
(rc=0)
[admin@R1] > /user add name=tehnicar group=read password=Tehnicar8
(rc=0)
[admin@R1] > /interface vlan add name=vlan10 vlan-id=10 interface=ether2
(rc=0)
[admin@R1] > /interface vlan add name=vlan99 vlan-id=99 interface=ether2
(rc=0)
[admin@R1] > /ip address add address=10.10.10.1/24 interface=vlan10
(rc=0)
[admin@R1] > /ip address add address=10.10.20.1/24 interface=ether3
(rc=0)
[admin@R1] > /ip address add address=10.10.99.1/24 interface=vlan99
(rc=0)
[admin@R1] > /ip address print
Flags: D - DYNAMIC
Columns: ADDRESS, NETWORK, INTERFACE
#   ADDRESS             NETWORK        INTERFACE
0 D 192.168.122.175/24  192.168.122.0  ether1
1   10.10.10.1/24       10.10.10.0     vlan10
2   10.10.20.1/24       10.10.20.0     ether3
3   10.10.99.1/24       10.10.99.0     vlan99
(rc=0)
[admin@R1] > /ip dhcp-client print
Columns: INTERFACE, USE-PEER-DNS, ADD-DEFAULT-ROUTE, STATUS, ADDRESS
# INTERFACE  USE-PEER-DNS  ADD-DEFAULT-ROUTE  STATUS  ADDRESS
0 ether1     yes           yes                bound   192.168.122.175/24
(rc=0)
CHECK PASS: K4 R1 addresses 
CHECK PASS: K4 R1 dhcp-client ether1 bound 
[admin@R1] > /user print
Columns: NAME, GROUP, INACTIVITY-POLICY
# NAME      GROUP  INACTIVITY-POLICY
;;; system default user
0 admin     full   none
1 tehnicar  read   none
(rc=0)
CHECK PASS: K4 R1 user tehnicar group read 
[admin@R1] > /system identity print
  name: R1
(rc=0)
CHECK PASS: K4 R1 identity R1 

##### Korak 5 – R1: DHCP, NAT, firewall, SNMP, log, backup
[admin@R1] > /ip pool add name=pool10 ranges=10.10.10.100-10.10.10.199
(rc=0)
[admin@R1] > /ip dhcp-server add name=dhcp10 interface=vlan10 address-pool=pool10
/ip dhcp-server add name=dhcp10 interface=vlan10 address-pool=pool[admin@R1] > /ip dhcp-server add name=dhcp10 interface=vlan10 address-pool=poo><cp10 interface=vlan10 address-pool=pool1                                      <cp10 interface=vlan10 address-pool=pool10<cp10 interface=vlan10 address-pool=pool10
(rc=0)
[admin@R1] > /ip dhcp-server network add address=10.10.10.0/24 gateway=10.10.10.1 dns-server=1.1.1.1
/ip dhcp-server network add address=10.10.10.0/24 gateway=10.10.10[admin@R1] > /ip dhcp-server network add address=10.10.10.0/24 gateway=10.10.1>< address=10.10.10.0/24 gateway=10.10.10.                                      < address=10.10.10.0/24 gateway=10.10.10.1 dns-server=1.1.1.1< address=10.10.10.0/24 gateway=10.10.10.1 dns-server=1.1.1.1
(rc=0)
[admin@R1] > /ip firewall nat add chain=srcnat out-interface=ether1 action=masquerade
/ip firewall nat add chain=srcnat out-interface=ether1 action=masq[admin@R1] > /ip firewall nat add chain=srcnat out-interface=ether1 action=mas><srcnat out-interface=ether1 action=masqu                                      <srcnat out-interface=ether1 action=masquerade<srcnat out-interface=ether1 action=masquerade
(rc=0)
[admin@R1] > /ip firewall filter add chain=forward in-interface=!vlan99 dst-address=10.10.99.0/24 action=drop
/ip firewall filter add chain=forward in-interface=!vlan99 dst-add[admin@R1] > /ip firewall filter add chain=forward in-interface=!vlan99 dst-ad><in=forward in-interface=!vlan99 dst-addr                                      <in=forward in-interface=!vlan99 dst-address=10.10.99.0/24 action=drop<in=forward in-interface=!vlan99 dst-address=10.10.99.0/24 action=drop
(rc=0)
[admin@R1] > /snmp community set [find default=yes] name=lab8 addresses=10.10.99.0/24
                                                          [admin@R1] > /snmp community set [find default=yes] name=lab8 addresses=10.10.9[admin@R1] > /snmp community set [find default=yes] name=lab8 addresses=10.10.><efault=yes] name=lab8 addresses=10.10.99                                      <efault=yes] name=lab8 addresses=10.10.99.0/24<efault=yes] name=lab8 addresses=10.10.99.0/24
(rc=0)
[admin@R1] > /snmp set enabled=yes location="Server sala" contact=admin@firma.rs
/snmp set enabled=yes location="Server sala" contact=admin@firma.r[admin@R1] > /snmp set enabled=yes location="Server sala" contact=admin@firma.><ion="Server sala" contact=admin@firma.rs                                      <ion="Server sala" contact=admin@firma.rs<ion="Server sala" contact=admin@firma.rs
(rc=0)
[admin@R1] > /system logging action add name=nadzor target=remote remote=10.10.99.10
/system logging action add name=nadzor target=remote remote=10.10.[admin@R1] > /system logging action add name=nadzor target=remote remote=10.10><name=nadzor target=remote remote=10.10.9                                      <name=nadzor target=remote remote=10.10.99.10<name=nadzor target=remote remote=10.10.99.10
(rc=0)
[admin@R1] > /system logging add topics=info action=nadzor
(rc=0)
[admin@R1] > /system logging add topics=warning action=nadzor
(rc=0)
[admin@R1] > /system logging add topics=error action=nadzor
(rc=0)
[admin@R1] > /system backup save name=R1-v8
Saving system configuration
Configuration backup saved
(rc=0)
[admin@R1] > /export file=R1-v8
(rc=0)
[admin@R1] > /ip firewall nat print
Flags: X - disabled, I - invalid; D - dynamic
 0    chain=srcnat action=masquerade out-interface=ether1
(rc=0)
[admin@R1] > /ip firewall filter print
Flags: X - disabled, I - invalid; D - dynamic
 0    chain=forward action=drop dst-address=10.10.99.0/24 in-interface=!vlan99
(rc=0)
[admin@R1] > /snmp community print
Flags: * - DEFAULT
Columns: NAME, ADDRESSES, SECURITY, READ-ACCESS, WRITE-ACCESS
#   NAME  ADDRESSES      SECURITY  READ-ACCESS  WRITE-ACCESS
0 * lab8  10.10.99.0/24  none      yes          no
(rc=0)
[admin@R1] > /system logging print where action=nadzor
Columns: TOPICS, ACTION
# TOPICS   ACTION
0 info     nadzor
1 warning  nadzor
2 error    nadzor
(rc=0)
[admin@R1] > /file print
 # NAME                              TYPE             SIZE LAST-MODIFIED
 0 R1-v8.backup                      backup        19.7KiB 2026-11-19 18:59:52
 1 skins                             directory             2026-11-19 18:59:24
 2 R1-v8.rsc                         script           1433 2026-11-19 18:59:52
(rc=0)
CHECK PASS: K5 R1 no command errors 
CHECK PASS: K5 R1-v8.backup and R1-v8.rsc exist 
CHECK PASS: K5 single community lab8 10.10.99.0/24 
[admin@R1] > /file print detail where name=R1-v8.rsc
 2 name=R1-v8.rsc type=script size=1433 last-modified=2026-11-19 18:59:52
   contents=
     # 2026-11-19 18:59:52 by RouterOS 7.16.2
     # software id =
     #
     /interface ethernet
     set [ find default-name=ether1 ] disable-running-check=no
     set [ find default-name=ether2 ] disable-running-check=no
     set [ find default-name=ether3 ] disable-running-check=no
     /interface vlan
     add interface=ether2 name=vlan10 vlan-id=10
     add interface=ether2 name=vlan99 vlan-id=99
     /ip pool
     add name=pool10 ranges=10.10.10.100-10.10.10.199
     /port
     set 0 name=serial0
     /snmp community
     set [ find default=yes ] addresses=10.10.99.0/24 name=lab8
     /system logging action
     add name=nadzor remote=10.10.99.10 target=remote
     /ip address
     add address=10.10.10.1/24 interface=vlan10 network=10.10.10.0
     add address=10.10.20.1/24 interface=ether3 network=10.10.20.0
     add address=10.10.99.1/24 interface=vlan99 network=10.10.99.0
     /ip dhcp-client
     add interface=ether1
     /ip dhcp-server
     add address-pool=pool10 interface=vlan10 name=dhcp10
     /ip dhcp-server network
     add address=10.10.10.0/24 dns-server=1.1.1.1 gateway=10.10.10.1
     /ip firewall filter
     add action=drop chain=forward dst-address=10.10.99.0/24 in-interface=!vlan99
     /ip firewall nat
     add action=masquerade chain=srcnat out-interface=ether1
     /snmp
     set contact=admin@firma.rs enabled=yes location="Server sala"
     /system identity
     set name=R1
     /system logging
     add action=nadzor topics=info
     add action=nadzor topics=warning
     add action=nadzor topics=error
     /system note
     set show-at-login=no
(rc=0)
 2 name=R1-v8.rsc type=script size=1433 last-modified=2026-11-19 18:59:52
   contents=
     # 2026-11-19 18:59:52 by RouterOS 7.16.2
     # software id =
     #
     /interface ethernet
     set [ find default-name=ether1 ] disable-running-check=no
     set [ find default-name=ether2 ] disable-running-check=no
     set [ find default-name=ether3 ] disable-running-check=no
     /interface vlan
     add interface=ether2 name=vlan10 vlan-id=10
     add interface=ether2 name=vlan99 vlan-id=99
     /ip pool
     add name=pool10 ranges=10.10.10.100-10.10.10.199
     /port
     set 0 name=serial0
     /snmp community
     set [ find default=yes ] addresses=10.10.99.0/24 name=lab8
     /system logging action
     add name=nadzor remote=10.10.99.10 target=remote
     /ip address
     add address=10.10.10.1/24 interface=vlan10 network=10.10.10.0
     add address=10.10.20.1/24 interface=ether3 network=10.10.20.0
     add address=10.10.99.1/24 interface=vlan99 network=10.10.99.0
     /ip dhcp-client
     add interface=ether1
     /ip dhcp-server
     add address-pool=pool10 interface=vlan10 name=dhcp10
     /ip dhcp-server network
     add address=10.10.10.0/24 dns-server=1.1.1.1 gateway=10.10.10.1
     /ip firewall filter
     add action=drop chain=forward dst-address=10.10.99.0/24 in-interface=!vlan99
     /ip firewall nat
     add action=masquerade chain=srcnat out-interface=ether1
     /snmp
     set contact=admin@firma.rs enabled=yes location="Server sala"
     /system identity
     set name=R1
     /system logging
     add action=nadzor topics=info
     add action=nadzor topics=warning
     add action=nadzor topics=error
     /system note
     set show-at-login=no

##### Korak 6 – VPC-ovi i provera
[VPC1> ip dhcp
DORA IP 10.10.10.199/24 GW 10.10.10.1
(rc=0)
[VPC2> ip 10.10.20.12/24 10.10.20.1
Checking for duplicate address...
VPC2 : 10.10.20.12 255.255.255.0 gateway 10.10.20.1
(rc=0)
[VPC1> ping 10.10.20.12 -c 2
84 bytes from 10.10.20.12 icmp_seq=1 ttl=63 time=1.778 ms
84 bytes from 10.10.20.12 icmp_seq=2 ttl=63 time=1.332 ms
(rc=0)
[VPC1> ping 8.8.8.8 -c 2
84 bytes from 8.8.8.8 icmp_seq=1 ttl=63 time=0.734 ms
84 bytes from 8.8.8.8 icmp_seq=2 ttl=63 time=0.923 ms
(rc=0)
[VPC1> ping 10.10.99.1 -c 2
84 bytes from 10.10.99.1 icmp_seq=1 ttl=64 time=0.655 ms
84 bytes from 10.10.99.1 icmp_seq=2 ttl=64 time=0.659 ms
(rc=0)
[VPC1> ping 10.10.99.2 -c 2
10.10.99.2 icmp_seq=1 timeout
10.10.99.2 icmp_seq=2 timeout
(rc=0)
[VPC2> ping 10.10.10.199 -c 2
84 bytes from 10.10.10.199 icmp_seq=1 ttl=63 time=0.961 ms
84 bytes from 10.10.10.199 icmp_seq=2 ttl=63 time=1.205 ms
(rc=0)
[admin@R1] > /ip neighbor print
Columns: INTERFACE, ADDRESS, MAC-ADDRESS, IDENTITY
#  INTERFACE  ADDRESS   MAC-ADDRESS        IDENTITY
0  ether2     10.1.0.1  22:E6:10:83:7D:FD  SW1
1  ether3     10.1.0.1  22:85:F2:37:71:75  SW2
(rc=0)
admin@SW1:~$ show lldp table
Capability codes: (R) Router, (B) Bridge, (O) Other
LocalPort    RemoteDevice    RemotePortID    Capability    RemotePortDescr
-----------  --------------  --------------  ------------  -----------------
Ethernet0    R1              ether2          R             ether2
Ethernet0    R1              vlan10          R             vlan10
Ethernet0    R1              vlan99          R             vlan99
Ethernet4    SW2             fortyGigE0/4    BR            Ethernet4
--------------------------------------------------
Total entries displayed:  4
(rc=0)
CHECK PASS: K6 VPC1 DHCP from pool, GW 10.10.10.1 
CHECK PASS: K6 VPC1 -> VPC2 ttl=63 
CHECK PASS: K6 VPC1 -> 8.8.8.8 
CHECK PASS: K6 VPC1 -> 10.10.99.1 (input chain) ok 
CHECK PASS: K6 VPC1 -> 10.10.99.2 blocked 
CHECK PASS: K6 VPC2 -> VPC1 ttl=63 
CHECK PASS: K6 R1 LLDP neighbors on ether2 and ether3 

##### dodatne provere
SNMPv2-MIB::sysName.0 = STRING: R1
SNMPv2-MIB::sysLocation.0 = STRING: Server sala
SNMPv2-MIB::sysContact.0 = STRING: admin@firma.rs

CHECK PASS: X SNMP R1 lab8 from 10.10.99.10: sysName R1, location, contact 
Timeout: No Response from 10.10.99.1.

CHECK PASS: X SNMP R1 community public no longer works 
CHECK PASS: X server -> SW1 10.10.99.2 (VLAN 99 over trunk) 
SNMPv2-MIB::sysDescr.0 = STRING: SONiC Software Version: SONiC.master.1280150-71d15c250 - HwSku: Force10-S6000 - Distribution: Debian 12.15 - Kernel: 7.0.0-43-generic

CHECK PASS: X SNMP SW1 lab8 from 10.10.99.10 (snmpd docker-sonic-vs) 
CHECK PASS: X server -> SW2 10.10.99.3 (VLAN 99 over trunk) 
SNMPv2-MIB::sysDescr.0 = STRING: SONiC Software Version: SONiC.master.1280150-71d15c250 - HwSku: Force10-S6000 - Distribution: Debian 12.15 - Kernel: 7.0.0-43-generic

CHECK PASS: X SNMP SW2 lab8 from 10.10.99.10 (snmpd docker-sonic-vs) 
[admin@R1] > :log info "vezba8 syslog test"
(rc=0)
system,info log rule added by console:admin+cetw200h5000@ttyS0 (*5 = /system logging add action=nadzor topics=info)system,info log rule added by console:admin+cetw200h5000@ttyS0 (*6 = /system logging add action=nadzor topics=warning)system,info log rule added by console:admin+cetw200h5000@ttyS0 (*7 = /system logging add action=nadzor topics=error)dhcp,info dhcp10 assigned 10.10.10.199 for 00:50:79:66:68:51 VPC1script,info vezba8 syslog test
CHECK PASS: X syslog from R1 received at 10.10.99.10:514 
[VPC1> ping 8.8.8.8 -c 1
84 bytes from 8.8.8.8 icmp_seq=1 ttl=63 time=0.772 ms
(rc=0)
19:00:34.036971 IP 192.168.122.175 > 8.8.8.8: ICMP echo request, id 21063, seq 1, length 64
19:00:34.036991 IP 8.8.8.8 > 192.168.122.175: ICMP echo reply, id 21063, seq 1, length 64

tcpdump: verbose output suppressed, use -v[v]... for full protocol decode
listening on eth0, link-type EN10MB (Ethernet), snapshot length 262144 bytes
2 packets captured
2 packets received by filter
0 packets dropped by kernel

CHECK PASS: X NAT: echo request at NAT node has source = R1 ether1 address 
[info] v8_sw1 LLDP_ENTRY_TABLE: ['LLDP_ENTRY_TABLE:Ethernet0', 'LLDP_ENTRY_TABLE:Ethernet4']
    LLDP_ENTRY_TABLE:Ethernet0 ['R1', 'vlan99', 'vlan99', '']
    LLDP_ENTRY_TABLE:Ethernet4 ['SW2', 'fortyGigE0/4', 'Ethernet4', '']
[info] v8_sw2 LLDP_ENTRY_TABLE: ['LLDP_ENTRY_TABLE:Ethernet4', 'LLDP_ENTRY_TABLE:Ethernet0']
    LLDP_ENTRY_TABLE:Ethernet4 ['SW1', 'fortyGigE0/4', 'Ethernet4', '']
    LLDP_ENTRY_TABLE:Ethernet0 ['R1', 'ether3', 'ether3', '']

===== SUMMARY
PASS K3 sw1 VLAN members as in table
PASS K3 sw1 Vlan99 10.10.99.2/24, no 10.0.0.x on used ports
PASS K3 sw1 saved config_db.json: hostname, VLANs, SNMP community
PASS K3 sw2 VLAN members as in table
PASS K3 sw2 Vlan99 10.10.99.3/24, no 10.0.0.x on used ports
PASS K3 sw2 saved config_db.json: hostname, VLANs, SNMP community
PASS K4 R1 addresses
PASS K4 R1 dhcp-client ether1 bound
PASS K4 R1 user tehnicar group read
PASS K4 R1 identity R1
PASS K5 R1 no command errors
PASS K5 R1-v8.backup and R1-v8.rsc exist
PASS K5 single community lab8 10.10.99.0/24
PASS K6 VPC1 DHCP from pool, GW 10.10.10.1
PASS K6 VPC1 -> VPC2 ttl=63
PASS K6 VPC1 -> 8.8.8.8
PASS K6 VPC1 -> 10.10.99.1 (input chain) ok
PASS K6 VPC1 -> 10.10.99.2 blocked
PASS K6 VPC2 -> VPC1 ttl=63
PASS K6 R1 LLDP neighbors on ether2 and ether3
PASS X SNMP R1 lab8 from 10.10.99.10: sysName R1, location, contact
PASS X SNMP R1 community public no longer works
PASS X server -> SW1 10.10.99.2 (VLAN 99 over trunk)
PASS X SNMP SW1 lab8 from 10.10.99.10 (snmpd docker-sonic-vs)
PASS X server -> SW2 10.10.99.3 (VLAN 99 over trunk)
PASS X SNMP SW2 lab8 from 10.10.99.10 (snmpd docker-sonic-vs)
PASS X syslog from R1 received at 10.10.99.10:514
PASS X NAT: echo request at NAT node has source = R1 ether1 address
28/28 PASS
```
