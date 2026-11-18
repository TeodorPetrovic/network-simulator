#!/bin/bash
# Vežba 7 test topologija: SW1/SW2 = gns3/openvswitch, VPC1/VPC2 = pravi vpcs binarni fajl simulatora,
# v7_router = Linux (alpine + iproute2) kao ZAMENA za MikroTik (MikroTik slika nije dostupna):
# VLAN podinterfejsi eth0.10/eth0.20 na jednom kablu, ip_forward=1. RouterOS komande iz rešenja se NE izvršavaju.
# VPCS radi kao u GNS3: UDP tunel (vpcs -s/-c) ka ubridge-u, koji ga premošćava na eth0 kontejnera (raw socket).
# Kontejner v7_vpcN služi samo kao "kabl": eth0 nema IP adresu, sve okvire šalje i prima VPCS.
# MAC adrese VPCS-a: 00:50:79:66:68:XX, XX = -m (simulator: poslednji bajt UUID-a čvora). V7_M1/V7_M2 ih biraju.
set -e
BIN=${V7_BIN:-$HOME/.local/share/network-simulator/bin}
M1=${V7_M1:-28}; M2=${V7_M2:-167}
docker image inspect v7-tcpdump:local >/dev/null 2>&1 || printf 'FROM alpine:latest\nRUN apk add --no-cache tcpdump\n' | docker build -q -t v7-tcpdump:local - >/dev/null
docker image inspect v7-router:local >/dev/null 2>&1 || printf 'FROM alpine:latest\nRUN apk add --no-cache iproute2\n' | docker build -q -t v7-router:local - >/dev/null
# kablovi: n1 = SW1 eth0 - VPC1, n2 = SW1 eth1 - VPC2, n3 = SW1 eth2 - SW2 eth0, n4 = SW2 eth1 - ruter (ether1 / eth0)
for n in 1 2 3 4; do docker network create --subnet 172.31.7$n.0/24 v7_n$n >/dev/null; done
docker run -d --name v7_sw1 --hostname SW1 --privileged --network v7_n1 --entrypoint sleep gns3/openvswitch:latest infinity >/dev/null
docker network connect v7_n2 v7_sw1
docker network connect v7_n3 v7_sw1
docker run -d --name v7_sw2 --hostname SW2 --privileged --network v7_n3 --entrypoint sleep gns3/openvswitch:latest infinity >/dev/null
docker network connect v7_n4 v7_sw2
docker run -d --name v7_router --hostname R1 --privileged --network v7_n4 \
  --sysctl net.ipv4.ip_forward=1 --sysctl net.ipv6.conf.all.disable_ipv6=1 v7-router:local sleep infinity >/dev/null
docker exec v7_router sh -c 'ip addr flush dev eth0; ip route flush table main'
for i in 1 2; do
  m=$([ $i = 1 ] && echo $M1 || echo $M2)
  docker run -d --name v7_vpc$i --hostname VPC$i --privileged --network v7_n$i \
    --sysctl net.ipv6.conf.all.disable_ipv6=1 \
    -v $BIN/vpcs:/usr/local/bin/vpcs:ro -v $BIN/ubridge:/usr/local/bin/ubridge:ro \
    -v /usr/lib/x86_64-linux-gnu:/usr/lib/x86_64-linux-gnu:ro -v /lib64:/lib64:ro \
    alpine:latest sleep infinity >/dev/null
  docker exec v7_vpc$i sh -c "ln -s /usr/lib/x86_64-linux-gnu /lib/x86_64-linux-gnu; ip addr flush dev eth0; ip route flush table main
    mkdir -p /v && cd /v
    printf '[bridge0]\nsource_udp = 20001:127.0.0.1:20000\ndestination_linux_raw = eth0\n' > ubridge.ini
    printf 'set pcname VPC$i\n' > startup.vpc
    (ubridge -f /v/ubridge.ini > ubridge.log 2>&1 &)
    (vpcs -p 5000 -m $m -i 1 -F -R -s 20000 -c 20001 -t 127.0.0.1 startup.vpc > vpcs.log 2>&1 &)"
done
for i in eth0 eth1 eth2; do docker exec v7_sw1 ip addr flush dev $i; done
for i in eth0 eth1; do docker exec v7_sw2 ip addr flush dev $i; done
docker exec v7_sw1 /etc/openvswitch/init.sh >/dev/null 2>&1 || true
docker exec v7_sw2 /etc/openvswitch/init.sh >/dev/null 2>&1 || true
sleep 1
docker network inspect v7_n1 v7_n2 v7_n3 v7_n4 --format '{{.Name}} {{range .Containers}}{{.Name}}={{.MacAddress}} {{end}}'
docker exec v7_vpc1 grep -q 'Destination NIO listener' /v/ubridge.log && docker exec v7_vpc2 grep -q 'Destination NIO listener' /v/ubridge.log
echo "VPCS -m: VPC1=$M1 VPC2=$M2"
