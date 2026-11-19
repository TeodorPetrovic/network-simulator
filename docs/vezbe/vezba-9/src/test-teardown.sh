#!/bin/bash
# briše samo v9_* kontejnere i mreže, po tačnom imenu (slike ostaju)
for c in v9_sw1 v9_vpc1 v9_nagios v9_syslog v9_server1 v9_server2; do docker rm -f -v "$c" >/dev/null 2>&1; done
for n in v9_n0 v9_n1 v9_n2 v9_n3 v9_n4; do docker network rm "$n" >/dev/null 2>&1; done
rm -rf "${V9_STATE:-${TMPDIR:-/tmp}/v9-state}"
true
