#!/bin/bash
# Vežba 9 test topologija (za održavanje, nije deo vežbe). Imena kontejnera i mreža: v9_*.
# Kabl = Docker mreža v9_nK između SW1 ethK i jednog uređaja:
#   n0 NAGIOS, n1 SYSLOG, n2 SERVER1, n3 SERVER2, n4 VPC1.
# SW1 = gns3/openvswitch (init.sh stavlja sve ethN u br0, NORMAL = običan svič).
# VPC1 = pravi vpcs + ubridge iz simulatora (kao u Vežbi 7). NAGIOS/SYSLOG/SERVER1/SERVER2 pokreće
# run-test.py (Korak 2) sa init.sh shimom simulatora, posle upisa /etc/network/interfaces iz rešenja.
set -e
BIN=${V9_BIN:-$HOME/.local/share/network-simulator/bin}
docker image inspect v9-tools:local >/dev/null 2>&1 || \
  printf 'FROM alpine:3.21\nRUN apk add --no-cache iproute2 tcpdump\n' | docker build -q -t v9-tools:local - >/dev/null
for n in 0 1 2 3 4; do docker network create --subnet 172.31.9$n.0/24 v9_n$n >/dev/null; done
docker run -d --name v9_sw1 --hostname SW1 --privileged --network v9_n0 --entrypoint sleep \
  gns3/openvswitch:latest infinity >/dev/null
for n in 1 2 3 4; do docker network connect v9_n$n v9_sw1; done
for i in 0 1 2 3 4; do docker exec v9_sw1 ip addr flush dev eth$i; done
docker exec v9_sw1 /etc/openvswitch/init.sh >/dev/null 2>&1 || true
docker run -d --name v9_vpc1 --hostname VPC1 --privileged --network v9_n4 \
  --sysctl net.ipv6.conf.all.disable_ipv6=1 \
  -v $BIN/vpcs:/usr/local/bin/vpcs:ro -v $BIN/ubridge:/usr/local/bin/ubridge:ro \
  -v /usr/lib/x86_64-linux-gnu:/usr/lib/x86_64-linux-gnu:ro -v /lib64:/lib64:ro \
  alpine:3.21 sleep infinity >/dev/null
docker exec v9_vpc1 sh -c "ln -s /usr/lib/x86_64-linux-gnu /lib/x86_64-linux-gnu; ip addr flush dev eth0; ip route flush table main
  mkdir -p /v && cd /v
  printf '[bridge0]\nsource_udp = 20001:127.0.0.1:20000\ndestination_linux_raw = eth0\n' > ubridge.ini
  printf 'set pcname VPC1\n' > startup.vpc
  (ubridge -f /v/ubridge.ini > ubridge.log 2>&1 &)
  (vpcs -p 5000 -m 50 -i 1 -F -R -s 20000 -c 20001 -t 127.0.0.1 startup.vpc > vpcs.log 2>&1 &)"
sleep 1
docker exec v9_sw1 ovs-vsctl list-ports br0 | tr '\n' ' '; echo
docker exec v9_vpc1 grep -q 'Destination NIO listener' /v/ubridge.log && echo "VPC1 ubridge OK"
