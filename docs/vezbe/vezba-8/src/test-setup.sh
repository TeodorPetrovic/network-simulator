#!/bin/bash
# Vežba 8 test topologija (za održavanje, nije deo vežbe). Imena kontejnera: v8_*.
#   R1   = pravi MikroTik CHR (RouterOS 7.16.2) u QEMU/KVM unutar kontejnera v8_r1; ether1..3 = tap1..3,
#          premošćeni na veth kablove e1..e3. Konzola: serijski port na 127.0.0.1:5000 u kontejneru.
#   SW1, SW2 = docker-sonic-vs (SONiC VS, master) sa t1 početnom konfiguracijom kao SONiC-VS slika u GNS3
#          (IP adrese 10.0.0.x/31 na portovima); mrežni namespace drži kontejner v8_swNh, ethN -> Ethernet(4N-4).
#   VPC1, VPC2 = pravi vpcs + ubridge iz simulatora (kao u Vežbi 7).
#   v8_nat = zamena za GNS3 NAT čvor (udhcpd, 192.168.122.1, "internet" adresa 8.8.8.8 na lo).
#   v8_srv = server za nadzor iz Vežbe 9 (10.10.99.10): prima syslog (UDP 514) i radi snmpwalk.
# Kablovi su veth parovi napravljeni direktno u namespace-ovima (Docker mreža = Linux bridge ne propušta LLDP).
# Na veth krajevima je isključen "tx checksum offload": SONiC-VS i QEMU čitaju okvire raw socket-om, pa bi UDP
# paketi (SNMP, syslog) stizali sa nedovršenim checksum-om i bili odbačeni.
# CHR slika: https://download.mikrotik.com/routeros/7.16.2/chr-7.16.2.img.zip, raspakovana, putanja u V8_CHR.
set -e
HERE=$(cd "$(dirname "$0")" && pwd)
BIN=${V8_BIN:-$HOME/.local/share/network-simulator/bin}
CHR=${V8_CHR:?V8_CHR = putanja do chr-7.16.2.img}
CHRDIR=$(cd "$(dirname "$CHR")" && pwd); CHRF=$(basename "$CHR")
docker image inspect v8-qemu:local >/dev/null 2>&1 || printf 'FROM alpine:3.21\nRUN apk add --no-cache qemu-system-x86_64 iproute2 socat busybox-extras\n' | docker build -q -t v8-qemu:local - >/dev/null
docker image inspect v8-tools:local >/dev/null 2>&1 || printf 'FROM alpine:3.21\nRUN apk add --no-cache tcpdump net-snmp-tools socat busybox-extras iproute2 ethtool util-linux\n' | docker build -q -t v8-tools:local - >/dev/null
docker image inspect docker-sonic-vs:latest >/dev/null

pid() { docker inspect -f '{{.State.Pid}}' "$1"; }
cable() { # cable ctrA ifA ctrB ifB
  docker run --rm --privileged --pid=host --net=host v8-tools:local \
    sh -c "ip link add $2 netns $(pid $1) type veth peer name $4 netns $(pid $3)
      nsenter -t $(pid $1) -n ethtool -K $2 tx off >/dev/null; nsenter -t $(pid $3) -n ethtool -K $4 tx off >/dev/null"
  docker exec "$1" ip link set "$2" up; docker exec "$3" ip link set "$4" up
}
run() { local n=$1; shift; docker run -d --name "$n" --hostname "${n#v8_}" --network none "$@" >/dev/null; }

run v8_r1 --privileged --device /dev/kvm -v "$CHRDIR:/img:ro" v8-qemu:local sleep infinity
run v8_sw1h --privileged v8-tools:local sleep infinity
run v8_sw2h --privileged v8-tools:local sleep infinity
run v8_nat --privileged v8-tools:local sleep infinity
run v8_srv --privileged v8-tools:local sleep infinity
for i in 1 2; do
  run v8_vpc$i --privileged -v $BIN/vpcs:/usr/local/bin/vpcs:ro -v $BIN/ubridge:/usr/local/bin/ubridge:ro \
    -v /usr/lib/x86_64-linux-gnu:/usr/lib/x86_64-linux-gnu:ro -v /lib64:/lib64:ro alpine:latest sleep infinity
done
# kablovi prema tabeli iz zadatka
cable v8_r1 e1 v8_nat eth0          # R1 ether1 - NAT
cable v8_r1 e2 v8_sw1h eth1         # R1 ether2 - SW1 Ethernet0
cable v8_r1 e3 v8_sw2h eth1         # R1 ether3 - SW2 Ethernet0
cable v8_sw1h eth2 v8_sw2h eth2     # SW1 Ethernet4 - SW2 Ethernet4
cable v8_sw1h eth3 v8_vpc1 eth0     # SW1 Ethernet8 - VPC1
cable v8_sw2h eth3 v8_vpc2 eth0     # SW2 Ethernet8 - VPC2
cable v8_sw2h eth4 v8_srv eth0      # SW2 Ethernet12 - server za nadzor
for h in v8_sw1h v8_sw2h; do docker exec $h sh -c 'ip link add eth0 type dummy && ip link set eth0 up'; done

# SONiC: t1 početna konfiguracija (kao sonic-vs.img u GNS3), samo za postojeće portove.
# docker i systemctl ne postoje u docker-sonic-vs (na pravom SONiC-u postoje): zamenjeni skriptama
# sonic-stub-* (inače "config vlan add" / "config snmp ..." upišu konfiguraciju pa padnu pri restartu servisa). BGPD_GRACEFUL_RESTART_ARG fali u slici.
for i in 1 2; do
  ports='0|4|8'; [ $i = 2 ] && ports='0|4|8|12'
  docker run -d --name v8_sw$i --privileged --network container:v8_sw${i}h -e BGPD_GRACEFUL_RESTART_ARG= \
    -v "$HERE/sonic-stub-docker:/usr/local/bin/docker:ro" -v "$HERE/sonic-stub-systemctl:/usr/local/bin/systemctl:ro" \
    --entrypoint bash docker-sonic-vs:latest -c \
    "mkdir -p /var/sonic; cd /usr/share/sonic/device/x86_64-kvm_x86_64-r0/Force10-S6000
     grep -E '^#|^Ethernet($ports)\s' port_config.ini > /tmp/pc.ini
     sonic-cfggen -H -k Force10-S6000 -p /tmp/pc.ini --preset t1 > /var/sonic/config_db.json
     cd /; exec /usr/local/bin/supervisord" >/dev/null
done

# R1: MikroTik CHR
docker exec v8_r1 sh -c 'for n in 1 2 3; do
    ip link add br$n type bridge; ip tuntap add tap$n mode tap
    ip link set tap$n master br$n up; ip link set e$n master br$n; ip link set br$n up
    echo 16384 > /sys/class/net/br$n/bridge/group_fwd_mask   # propusti LLDP (01:80:c2:00:00:0e)
  done
  nics=""; for n in 1 2 3; do nics="$nics -netdev tap,id=n$n,ifname=tap$n,script=no,downscript=no -device virtio-net-pci,netdev=n$n,mac=0c:88:00:00:00:0$n"; done
  (qemu-system-x86_64 -enable-kvm -m 256 -smp 1 -snapshot -drive file=/img/'"$CHRF"',format=raw,if=virtio $nics \
    -serial telnet:127.0.0.1:5000,server,nowait -monitor none -display none > /tmp/qemu.log 2>&1 &)'

# NAT čvor (zamena): DHCP 192.168.122.100-200, gateway 192.168.122.1, 8.8.8.8 kao "internet"
docker exec v8_nat sh -c 'ip addr add 192.168.122.1/24 dev eth0; ip addr add 8.8.8.8/32 dev lo
  mkdir -p /var/lib/udhcpd; touch /var/lib/udhcpd/udhcpd.leases
  printf "start 192.168.122.100\nend 192.168.122.200\ninterface eth0\noption subnet 255.255.255.0\noption router 192.168.122.1\noption dns 192.168.122.1\n" > /etc/udhcpd.conf
  udhcpd /etc/udhcpd.conf'
# server za nadzor: syslog prijemnik
docker exec v8_srv sh -c 'ip addr add 10.10.99.10/24 dev eth0; ip route add default via 10.10.99.1
  (socat -u UDP-RECV:514 OPEN:/tmp/syslog.log,creat,append &)'

# VPCS (kao u GNS3): UDP tunel ka ubridge-u, koji ga premošćava na eth0 kontejnera
for i in 1 2; do
  m=$([ $i = 1 ] && echo 81 || echo 82)
  docker exec v8_vpc$i sh -c "ln -sf /usr/lib/x86_64-linux-gnu /lib/x86_64-linux-gnu; mkdir -p /v && cd /v
    printf '[bridge0]\nsource_udp = 20001:127.0.0.1:20000\ndestination_linux_raw = eth0\n' > ubridge.ini
    printf 'set pcname VPC$i\n' > startup.vpc
    (ubridge -f /v/ubridge.ini > ubridge.log 2>&1 &)
    (vpcs -p 5000 -m $m -i 1 -F -R -s 20000 -c 20001 -t 127.0.0.1 startup.vpc > vpcs.log 2>&1 &)"
done

# čekanje: SONiC portovi napravljeni
for i in 1 2; do
  for t in $(seq 1 120); do
    docker exec v8_sw$i redis-cli -n 0 exists PORT_TABLE:PortInitDone 2>/dev/null | grep -q 1 && break; sleep 2
  done
  docker exec v8_sw$i redis-cli -n 0 exists PORT_TABLE:PortInitDone | grep -q 1 || { echo "SW$i nije spreman"; exit 1; }
done
# lldpd u docker-sonic-vs-u izgubi svoj socket pri pokretanju: restart lldpd, pa lldpmgrd i lldp-syncd
for i in 1 2; do
  docker exec v8_sw$i bash -c 'supervisorctl restart lldpd; sleep 2; supervisorctl restart lldpmgrd lldp-syncd' >/dev/null
done
echo OK
