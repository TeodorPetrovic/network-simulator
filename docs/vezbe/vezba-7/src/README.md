# Vežba 7 – izgradnja PDF-ova i test

Izlaz: `../Vezba-7-zadatak.pdf` (samo zadatak, bez uvoda), `../Vezba-7-resenje.pdf` (zadatak + rešenje),
`../Vezba-7-kompletno.pdf` (uvod + zadatak + rešenje). Ulaz: `../task/{zadatak,resenje}.frag.html`,
`intro.frag.html`, `style.css`. Brojevi slika (`<p class="caption">Slika N.`) se u svakom PDF-u numerišu redom.

    cd docs/vezbe/vezba-7/src && node build.mjs

Zahteva `playwright-core` iz `/home/teodor/projects/learn-core/node_modules` i Chromium
`/home/teodor/.cache/ms-playwright/chromium-1248`. Font u kodu se nikad ne smanjuje: build javlja grešku ako je neka
linija u `pre` bloku duža od ~108 znakova (duge komande se prelamaju sa `\`).

Test rešenja (Docker `gns3/openvswitch` za SW1/SW2, pravi `vpcs` + `ubridge` iz
`~/.local/share/network-simulator/bin` za VPC1/VPC2; imena `v7_*`). MikroTik slika nije dostupna: `v7_router`
(alpine + iproute2, `eth0.10`/`eth0.20`, `ip_forward=1`) je Linux ZAMENA za MikroTik; RouterOS komande iz rešenja
(blok `data-dev="mikrotik"`) se ne izvršavaju i nisu proverene.

    ./test-teardown.sh; ./test-setup.sh && python3 run-test.py; ./test-teardown.sh

`V7_M1`/`V7_M2` biraju MAC adrese VPC-ova (`-m`). `V7_LEAK30=1` je samo-provera harness-a (SW2 propušta VLAN 30,
NEG provere moraju da padnu). Rezultat i pun izlaz: `../test-log.md`.
