# Vežba 7 – izgradnja PDF-ova i test

Izlaz: `../Vezba-7-zadatak.pdf` (uvod + zadatak), `../Vezba-7-resenje.pdf` (zadatak + rešenje),
`../Vezba-7-kompletno.pdf` (uvod + zadatak + rešenje). Ulaz: `../task/{zadatak,resenje}.frag.html`,
`intro.frag.html`, `style.css`.

    cd docs/vezbe/vezba-7/src && node build.mjs

Zahteva `playwright-core` iz `/home/teodor/projects/learn-core/node_modules` i Chromium
`/home/teodor/.cache/ms-playwright/chromium-1248`. Font u kodu se nikad ne smanjuje: build javlja grešku ako je neka
linija u `pre` bloku duža od ~108 znakova (duge komande se prelamaju sa `\`).

Test rešenja na pravim kontejnerima (Docker, `gns3/openvswitch` + `alpine`, imena `v7_*`):

    ./test-teardown.sh; ./test-setup.sh && python3 run-test.py; ./test-teardown.sh

Rezultat i pun izlaz: `../test-log.md`.
