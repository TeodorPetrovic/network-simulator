// Gradi ../Vezba-6-kompletno.pdf iz fragmenata. Pokretanje: node build.mjs
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

// Smanji font u pre blokovima kojima je neka linija preduga (komande se nikad ne lome)
const CH = 0.602, AVAIL_PT = 492;
function fitPre(html) {
  return html.replace(/<pre([^>]*)>([\s\S]*?)<\/pre>/g, (m, attrs, body) => {
    const text = body.replace(/<[^>]+>/g, '').replace(/&lt;/g, '<').replace(/&gt;/g, '>').replace(/&amp;/g, '&').replace(/&quot;/g, '"');
    const max = Math.max(...text.split('\n').map((l) => l.length));
    const fs_ = Math.min(7.5, AVAIL_PT / (CH * max));
    if (fs_ >= 7.5) return m;
    return `<pre${attrs} style="font-size:${fs_.toFixed(2)}pt">${body}</pre>`;
  });
}

const cover = `<div class="cover">
<h2>Vežba 6 – OVS kao ruter i praćenje konekcija</h2>
<p>Ova vežba ima dva zadatka. Svaki zadatak je odmah praćen svojim rešenjem. U Zadatku 1 jedan Open vSwitch radi kao ruter između dve IP podmreže, bez ijednog <code>NORMAL</code> pravila: ARP odgovara, MAC adrese menja i TTL smanjuje sam OpenFlow. U Zadatku 2 OVS štiti unutrašnju mrežu praćenjem konekcija (conntrack): veze koje otvara unutrašnji računar prolaze zajedno sa odgovorima, a veze koje otvara spoljašnji računar se blokiraju. Pre zadataka sledi kratak uvod u nove pojmove.</p>
<h2 style="font-size:13pt;border:none;margin-top:18px">Šta je novo u ovoj vežbi</h2>
${fs.readFileSync(path.join(here, "intro.frag.html"), "utf8")}
</div>`;

const parts = [
  cover,
  rd('task1/zadatak.frag.html'),
  rd('task1/resenje.frag.html'),
  rd('task2/zadatak.frag.html'),
  rd('task2/resenje.frag.html'),
];
const body = parts.map((p, i) => i === 0 ? p : `<section class="page">${p}</section>`).join('\n');

const html = `<!doctype html><html lang="sr"><head><meta charset="utf-8"><title>Vežba 6</title><style>${css}</style></head><body>${fitPre(body)}</body></html>`;
fs.writeFileSync(path.join(here, 'build.html'), html);

const browser = await chromium.launch({ executablePath: '/home/teodor/.cache/ms-playwright/chromium-1248/chrome-linux64/chrome' });
const page = await browser.newPage();
await page.setContent(html, { waitUntil: 'load' });
await page.pdf({
  path: path.join(root, 'Vezba-6-kompletno.pdf'),
  format: 'Letter', printBackground: true, preferCSSPageSize: true,
  displayHeaderFooter: true,
  headerTemplate: '<div style="font-size:7pt;font-family:DejaVu Sans;color:#777;width:100%;padding:0 0.7in;text-align:right">Vežba 6</div>',
  footerTemplate: '<div style="font-size:8pt;font-family:DejaVu Sans;color:#777;width:100%;text-align:center"><span class="pageNumber"></span> / <span class="totalPages"></span></div>',
});
await browser.close();
console.log('OK');
