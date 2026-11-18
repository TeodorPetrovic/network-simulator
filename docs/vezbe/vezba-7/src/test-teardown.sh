#!/bin/bash
# briše samo v7_* kontejnere i mreže, po tačnom imenu
for c in v7_sw1 v7_sw2 v7_router v7_vpc1 v7_vpc2 v7_pc1 v7_pc2 v7_arping \
  v7_inj v7_cap_sw2eth0 v7_cap_sw2eth1 v7_cap_vpc1 v7_cap_vpc2 v7_cap_pc1 v7_cap_pc2; do docker rm -f "$c" >/dev/null 2>&1; done
for n in v7_n1 v7_n2 v7_n3 v7_n4; do docker network rm "$n" >/dev/null 2>&1; done
true
