# Vežba 5 – izgradnja PDF-a

Izlaz: `../Vezba-5-kompletno.pdf`. Ulaz: `../task1/{zadatak,resenje}.frag.html`, `../task2/{zadatak,resenje}.frag.html`, `intro.frag.html` (uvod), `style.css`. Fajlovi `test-log.md` se ne koriste.

Rebuild posle izmene fragmenata:

    cd docs/vezbe/vezba-5/src && node build.mjs

Zahteva `playwright-core` iz `/home/teodor/projects/learn-core/node_modules` i Chromium `/home/teodor/.cache/ms-playwright/chromium-1248` (putanje su na vrhu `build.mjs`). `build.html` je međufajl (može se obrisati). Pre blokovi sa predugim linijama automatski dobijaju manji font, a komande se nikad ne lome.
