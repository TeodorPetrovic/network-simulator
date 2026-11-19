# Vežba 9 – izgradnja PDF-ova i test

Izlaz: `../Vezba-9-zadatak.pdf` (samo zadatak, bez uvoda), `../Vezba-9-resenje.pdf` (zadatak + rešenje),
`../Vezba-9-kompletno.pdf` (uvod o Nagios/syslog/SNMP + zadatak + rešenje). Ulaz: `../task/{zadatak,resenje}.frag.html`,
`intro.frag.html`, `style.css`. Brojevi slika (`<p class="caption">Slika N.`) se u svakom PDF-u numerišu redom.

    cd docs/vezbe/vezba-9/src && node build.mjs

Zahteva `playwright-core` iz `/home/teodor/projects/learn-core/node_modules` i Chromium
`/home/teodor/.cache/ms-playwright/chromium-1248`. Font u kodu se nikad ne smanjuje: build javlja grešku ako je neka
linija u `pre` bloku duža od ~108 znakova (duge komande se prelamaju sa `\`).

## Test rešenja

Slike: `jasonrivers/nagios` (Nagios Core 4.5.12, Ubuntu 24.04), `vezba9-linux` (pravi se iz `Dockerfile` bloka u
zadatku), `gns3/openvswitch` za SW1, pravi `vpcs` + `ubridge` iz `~/.local/share/network-simulator/bin` za VPC1.
Imena kontejnera i mreža: `v9_*`. R1 (MikroTik) i SONiC nisu dostupni: njihovi blokovi (`data-dev="mikrotik"`,
`"sonic"`) se ne izvršavaju, a u Nagios-u su ti hostovi u testu DOWN.

    export TMPDIR=<scratch>; ./test-teardown.sh; ./test-setup.sh && python3 run-test.py; ./test-teardown.sh

- `test-setup.sh` – kablovi = Docker mreže `v9_n0..n4` (SW1 eth0..eth4 – NAGIOS, SYSLOG, SERVER1, SERVER2, VPC1),
  SW1 (svi portovi u `br0`), VPC1.
- `run-test.py` – izvršava blokove iz `task/resenje.frag.html` doslovno (vidi docstring). NAGIOS/SYSLOG/SERVER1/2 se
  pokreću sa init.sh shimom simulatora (`src/runtime/resources/init.sh`, BusyBox `ifup` primenjuje
  `/etc/network/interfaces` iz bloka `data-netcfg`). NAGIOS komande idu kroz pomoćnu konzolu
  (`/gns3/bin/busybox sh`), ostale kroz `/bin/sh` slike. Stanje Nagios-a se proverava preko `statusjson.cgi`.
  Korak 7: `docker stop v9_server2` = Stop u GNS3.
- Samo za Docker test (nije deo vežbe): Docker IPAM adresa i default ruta se brišu sa eth0, a TX checksum offload
  se isključuje (`ethtool -K eth0 tx off`, slika `v9-tools:local`), jer userspace OVS (netdev) ne dopunjuje delimične
  TCP/UDP checksume sa veth-a pa bez toga radi samo ping.
- `test-teardown.sh` – briše samo `v9_*` kontejnere (sa anonimnim volumenima) i mreže.

Rezultat i pun izlaz: `../test-log.md`.
