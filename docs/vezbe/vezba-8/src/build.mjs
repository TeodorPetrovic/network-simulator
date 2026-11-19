// Gradi tri PDF-a Vežbe 8 iz fragmenata. Pokretanje: node build.mjs
//   ../Vezba-8-zadatak.pdf    samo zadatak (bez uvoda i rešenja)
//   ../Vezba-8-resenje.pdf    zadatak + rešenje
//   ../Vezba-8-kompletno.pdf  uvod + zadatak + rešenje
import fs from 'fs';
import path from 'path';
import { fileURLToPath } from 'url';
import { createRequire } from 'module';
const require = createRequire('/home/teodor/projects/learn-core/node_modules/');
const { chromium } = require('playwright-core');

const here = path.dirname(fileURLToPath(import.meta.url));
const root = path.join(here, '..');
const rd = (p) => fs.readFileSync(path.join(root, p), 'utf8');
const css = fs.readFileSync(path.join(here, 'style.css'), 'utf8');

// Font u pre blokovima se nikad ne smanjuje: preduga linija je greška u fragmentu
// (duge komande se prelamaju znakom \ na kraju reda).
const CH = 0.602, AVAIL_PT = 492, FONT_PT = 7.5, MAX_CHARS = Math.floor(AVAIL_PT / (CH * FONT_PT));
function checkPre(html, name) {
  for (const [, body] of html.matchAll(/<pre[^>]*>([\s\S]*?)<\/pre>/g)) {
    const text = body.replace(/<[^>]+>/g, '').replace(/&lt;/g, '<').replace(/&gt;/g, '>').replace(/&amp;/g, '&').replace(/&quot;/g, '"');
    for (const line of text.split('\n')) {
      if (line.length > MAX_CHARS) throw new Error(`${name}: linija duža od ${MAX_CHARS} znakova (${line.length}): ${line}`);
    }
  }
}

const cover = `<div class="cover">
<h2>Vežba 8 – MikroTik ruter i SONiC svičevi</h2>
<p>Ova vežba ima jedan zadatak: mala mreža firme sa MikroTik ruterom (RouterOS CHR) i dva SONiC sviča, sa VLAN-ovima po organizacionim jedinicama, rutiranjem između VLAN-ova, DHCP-om, NAT-om, firewall pravilom i pripremom uređaja za nadzor (SNMP, slanje loga) koji se nastavlja u Vežbi 9. Pre zadatka sledi kratak uvod u MikroTik RouterOS i SONiC.</p>
<h2 style="font-size:13pt;border:none;margin-top:18px">Šta je novo u ovoj vežbi</h2>
${fs.readFileSync(path.join(here, 'intro.frag.html'), 'utf8')}
</div>`;

const task = rd('task/zadatak.frag.html');
const sol = rd('task/resenje.frag.html');
const docs = {
  'Vezba-8-zadatak.pdf': [task],
  'Vezba-8-resenje.pdf': [task, sol],
  'Vezba-8-kompletno.pdf': [cover, task, sol],
};

const browser = await chromium.launch({ executablePath: '/home/teodor/.cache/ms-playwright/chromium-1248/chrome-linux64/chrome' });
for (const [file, parts] of Object.entries(docs)) {
  let body = parts.map((p) => (p === cover ? p : `<section class="page">${p}</section>`)).join('\n');
  // slike se numerišu redom u svakom dokumentu (zadatak bez uvoda počinje od Slike 1)
  let fig = 0;
  body = body.replace(/(<p class="caption">Slika )\d+\./g, (_, pre) => `${pre}${++fig}.`);
  checkPre(body, file);
  const html = `<!doctype html><html lang="sr"><head><meta charset="utf-8"><title>Vežba 8</title><style>${css}</style></head><body>${body}</body></html>`;
  fs.writeFileSync(path.join(here, 'build.html'), html);
  const page = await browser.newPage();
  await page.setContent(html, { waitUntil: 'load' });
  await page.pdf({
    path: path.join(root, file),
    format: 'Letter', printBackground: true, preferCSSPageSize: true,
    displayHeaderFooter: true,
    headerTemplate: '<div style="font-size:7pt;font-family:DejaVu Sans;color:#777;width:100%;padding:0 0.7in;text-align:right">Vežba 8</div>',
    footerTemplate: '<div style="font-size:8pt;font-family:DejaVu Sans;color:#777;width:100%;text-align:center"><span class="pageNumber"></span> / <span class="totalPages"></span></div>',
  });
  await page.close();
  console.log('OK', file);
}
await browser.close();
