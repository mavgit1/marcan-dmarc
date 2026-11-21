// Makes the docs' social preview image, docs/public/og.png (1200x630), from the logo mark,
// the name and the tagline, in the docs' light look. Run it again after a logo or tagline change:
//
//   pnpm install            (here, once: installs Playwright's Chromium)
//   pnpm --dir ../../docs install   (once: the Inter font comes from VitePress)
//   node og-image.mjs
//
// It writes ../../docs/public/og.png; look at the result before committing it.
import { chromium } from 'playwright';
import { readFile } from 'node:fs/promises';
import { fileURLToPath } from 'node:url';

const here = (path) => fileURLToPath(new URL(path, import.meta.url));
const logo = await readFile(here('../../docs/public/logo-light.svg'), 'utf8');
const font = await readFile(
	here('../../docs/node_modules/vitepress/dist/client/theme-default/fonts/inter-roman-latin.woff2')
);
const out = here('../../docs/public/og.png');

const html = `<!doctype html>
<html><head><meta charset="utf-8"><style>
@font-face { font-family: Inter; font-weight: 100 900; src: url(data:font/woff2;base64,${font.toString('base64')}) format('woff2'); }
* { margin: 0; box-sizing: border-box; }
html, body { width: 1200px; height: 630px; }
body {
	font-family: Inter, sans-serif; font-feature-settings: 'cv11', 'ss01';
	background: #ffffff; color: #171717;
	display: flex; flex-direction: column; justify-content: center; padding: 0 110px;
	border-bottom: 14px solid #155dfc;
}
.row { display: flex; align-items: center; gap: 56px; }
.mark svg { width: 200px; height: 200px; display: block; }
h1 { font-size: 132px; font-weight: 700; letter-spacing: -0.035em; line-height: 1; }
p { margin-top: 22px; font-size: 46px; font-weight: 500; letter-spacing: -0.015em; color: #525252; }
.url { position: absolute; left: 110px; bottom: 58px; font-size: 28px; font-weight: 500; color: #155dfc; }
</style></head><body>
<div class="row">
	<div class="mark">${logo.replace(/<!--[\s\S]*?-->/g, '')}</div>
	<div><h1>Marcan</h1><p>Self-hosted DMARC monitoring</p></div>
</div>
<div class="url">marcan.kaitiaki.tech</div>
</body></html>`;

const browser = await chromium.launch();
const page = await browser.newPage({ viewport: { width: 1200, height: 630 }, deviceScaleFactor: 1 });
await page.setContent(html);
await page.evaluate(() => document.fonts.ready);
await page.screenshot({ path: out });
await browser.close();
console.log(`wrote ${out}`);
