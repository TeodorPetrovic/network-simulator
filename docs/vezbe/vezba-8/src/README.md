# Vežba 8 – izgradnja PDF-ova i test

Izlaz: `../Vezba-8-zadatak.pdf` (samo zadatak, bez uvoda), `../Vezba-8-resenje.pdf` (zadatak + rešenje),
`../Vezba-8-kompletno.pdf` (uvod + zadatak + rešenje). Ulaz: `../task/{zadatak,resenje}.frag.html`,
`intro.frag.html`, `style.css`. Brojevi slika (`<p class="caption">Slika N.`) se u svakom PDF-u numerišu redom.

    cd docs/vezbe/vezba-8/src && node build.mjs

Zahteva `playwright-core` iz `/home/teodor/projects/learn-core/node_modules` i Chromium
`/home/teodor/.cache/ms-playwright/chromium-1248`. Font u kodu se nikad ne smanjuje: build javlja grešku ako je neka
linija u `pre` bloku duža od ~108 znakova (duge komande se prelamaju sa `\`).

Provera adresnog plana (tabele u zadatku i komande u rešenju): `python3 check-plan.py`.

## Test rešenja

- R1: pravi MikroTik CHR 7.16.2 (QEMU/KVM u kontejneru `v8_r1`), slika sa
  `https://download.mikrotik.com/routeros/7.16.2/chr-7.16.2.img.zip` (besplatna, bez naloga), raspakovana; putanja u
  `V8_CHR`. RouterOS komande se kucaju u serijsku konzolu (`ros.py`).
- SW1, SW2: `docker-sonic-vs:latest` (SONiC master, ~230 MB, iz
  `https://sonic-build.azurewebsites.net/api/sonic/artifacts?branchName=master&platform=vs&target=target/docker-sonic-vs.gz`,
  `gunzip -c | docker load`), sa t1 početnom konfiguracijom kao SONiC-VS VM. Nije SONiC-VS VM iz GNS3: nema
  systemd/hostcfgd (ime sviča harness primenjuje sam), `docker`/`systemctl` su zamenjeni skriptama `sonic-stub-*`.
- VPC1, VPC2: pravi `vpcs` + `ubridge` iz `~/.local/share/network-simulator/bin`.
- `v8_nat` (udhcpd, 192.168.122.1, 8.8.8.8 na lo) umesto GNS3 NAT čvora; `v8_srv` (10.10.99.10 na SW2 Ethernet12)
  kao server za nadzor: snmpget ka R1/SW1/SW2 i prijem syslog-a.

      ./test-teardown.sh; V8_CHR=/putanja/chr-7.16.2.img ./test-setup.sh && python3 run-test.py; ./test-teardown.sh

Rezultat i pun izlaz: `../test-log.md`.
