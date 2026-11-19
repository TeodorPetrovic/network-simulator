#!/bin/bash
# briše samo v8_* kontejnere, po tačnom imenu (kablovi su veth parovi i nestaju sa kontejnerima)
for c in v8_sw1 v8_sw2 v8_sw1h v8_sw2h v8_r1 v8_vpc1 v8_vpc2 v8_nat v8_srv; do docker rm -f "$c" >/dev/null 2>&1; done
true
