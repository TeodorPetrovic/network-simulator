#!/bin/bash
set -e
docker image inspect v7-tcpdump:local >/dev/null 2>&1 || printf 'FROM alpine:latest\nRUN apk add --no-cache tcpdump\n' | docker build -q -t v7-tcpdump:local - >/dev/null
# links: n1 = SW1 eth0 - PC1 eth0, n2 = SW1 eth1 - PC2 eth0, n3 = SW1 eth2 - SW2 eth0
docker network create --subnet 172.31.71.0/24 v7_n1 >/dev/null
docker network create --subnet 172.31.72.0/24 v7_n2 >/dev/null
docker network create --subnet 172.31.73.0/24 v7_n3 >/dev/null
docker run -d --name v7_sw1 --hostname SW1 --privileged --network v7_n1 --entrypoint sleep gns3/openvswitch:latest infinity >/dev/null
docker network connect v7_n2 v7_sw1
docker network connect v7_n3 v7_sw1
docker run -d --name v7_sw2 --hostname SW2 --privileged --network v7_n3 --entrypoint sleep gns3/openvswitch:latest infinity >/dev/null
docker run -d --name v7_pc1 --hostname PC1 --cap-add NET_ADMIN --network v7_n1 alpine:latest sleep infinity >/dev/null
docker run -d --name v7_pc2 --hostname PC2 --cap-add NET_ADMIN --network v7_n2 alpine:latest sleep infinity >/dev/null
for c in v7_sw1; do for i in eth0 eth1 eth2; do docker exec $c ip addr flush dev $i; done; done
docker exec v7_sw2 ip addr flush dev eth0
for c in v7_pc1 v7_pc2; do docker exec $c ip addr flush dev eth0; docker exec $c ip route flush table main 2>/dev/null || true; done
docker exec v7_sw1 /etc/openvswitch/init.sh >/dev/null 2>&1 || true
docker exec v7_sw2 /etc/openvswitch/init.sh >/dev/null 2>&1 || true
# verify eth order
for c in v7_sw1; do docker exec $c sh -c 'for i in eth0 eth1 eth2; do echo "$i $(cat /sys/class/net/$i/address)"; done'; done
docker network inspect v7_n1 v7_n2 v7_n3 --format '{{.Name}} {{range .Containers}}{{.Name}}={{.MacAddress}} {{end}}'
