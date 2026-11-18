# Vežba 7 – izgradnja PDF-ova i test

Izlaz: `../Vezba-7-zadatak.pdf` (samo zadatak, bez uvoda), `../Vezba-7-resenje.pdf` (zadatak + rešenje),
`../Vezba-7-kompletno.pdf` (uvod + zadatak + rešenje). Ulaz: `../task/{zadatak,resenje}.frag.html`,
`intro.frag.html`, `style.css`. Brojevi slika (`<p class="caption">Slika N.`) se u svakom PDF-u numerišu redom.

    cd docs/vezbe/vezba-7/src && node build.mjs

Zahteva `playwright-core` iz `/home/teodor/projects/learn-core/node_modules` i Chromium
`/home/teodor/.cache/ms-playwright/chromium-1248`. Font u kodu se nikad ne smanjuje: build javlja grešku ako je neka
linija u `pre` bloku duža od ~108 znakova (duge komande se prelamaju sa `\`).

Test rešenja na pravim uređajima (Docker `gns3/openvswitch` za SW1/SW2, pravi `vpcs` + `ubridge` iz
`~/.local/share/network-simulator/bin` za VPC1/VPC2; imena `v7_*`):

    ./test-teardown.sh; ./test-setup.sh && python3 run-test.py; ./test-teardown.sh

`V7_M1`/`V7_M2` biraju MAC adrese VPC-ova (`-m`). Rezultat i pun izlaz: `../test-log.md`.
