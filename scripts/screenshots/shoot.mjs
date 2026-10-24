// Takes the docs screenshots (docs/public/screenshots/<name>-light.png and -dark.png)
// from a running Marcan: 1440x900 at 2x, en-GB, light and dark from prefers-color-scheme.
//
//   node shoot.mjs <base-url> <out-dir> [<setup-base-url>] [--only name,name]
//
// <setup-base-url> is a second server on an empty data directory, for the setup's Mailbox
// step (its Organization step is confirmed as "Example IT" on the way); without it the
// setup shot is skipped. On the demo server nothing is saved or sent: forms are filled in
// the page only.
import { chromium } from 'playwright';
import { mkdir } from 'node:fs/promises';
import { join } from 'node:path';

const args = process.argv.slice(2);
const onlyAt = args.indexOf('--only');
const only = onlyAt >= 0 ? new Set(args.splice(onlyAt, 2)[1].split(',')) : null;
const [base, outDir, setupBase] = args;
if (!base || !outDir) {
	console.error('usage: node shoot.mjs <base-url> <out-dir> [<setup-base-url>] [--only name,name]');
	process.exit(2);
}
const url = (root, path) => new URL(path, root.endsWith('/') ? root : `${root}/`).href;

async function api(path) {
	const res = await fetch(url(base, path));
	if (!res.ok) throw new Error(`${path}: ${res.status}`);
	return res.json();
}

// The domain with issues: the busiest one at p=none with failures and DNS findings.
const domains = (await api('api/domains')).items;
const domain = domains
	.filter((d) => d.policy?.p === 'none' && d.delivered_failures > 0 && d.assessment.issues.length > 0)
	.sort((a, b) => b.total_messages - a.total_messages)[0];
// A clean domain for the DNS inspector: the best DNS score at p=reject, with mail.
const clean = domains
	.filter((d) => d.dns && d.policy?.p === 'reject' && !d.parked && d.total_messages > 0)
	.sort((a, b) => b.dns.score - a.dns.score || b.total_messages - a.total_messages)[0];
// The busy sender: the most mail in the last 30 days among those sending for three domains or more.
const sender = (await api('api/senders?since=30d')).items
	.filter((s) => s.domain_count >= 3)
	.sort((a, b) => b.total_messages - a.total_messages)[0];
// The client report: the largest organization.
const org = (await api('api/organizations')).items.sort((a, b) => b.total_messages - a.total_messages)[0];
if (!domain || !clean || !sender || !org) throw new Error('the data lacks a domain, a clean domain, a sender or an organization to show');

/** Each shot: its file name, the server, the path, and what to do before the picture. */
const SHOTS = [
	{ name: 'dashboard', path: '' },
	{ name: 'domains', path: 'domains' },
	{ name: 'domain', path: `domains/${domain.id}` },
	{ name: 'failures', path: `domains/${domain.id}?tab=failures&outcome=all_failures` },
	{ name: 'organizations', path: 'organizations' },
	{ name: 'senders', path: 'senders' },
	{ name: 'sender', path: `senders/${encodeURIComponent(sender.source_ip)}` },
	{ name: 'alerts', path: 'alerts' },
	{
		name: 'dns',
		path: 'tools/dns',
		async before(page) {
			await page.getByLabel('Domain', { exact: true }).fill(clean.domain);
			await page.getByRole('button', { name: 'Inspect' }).click();
			await page.getByText('Showing live checks for').waitFor();
		}
	},
	{
		name: 'sending',
		path: 'settings/sending',
		// Sending as a configured installation has it (on, a password stored), in this page's
		// answer only: the demo keeps it off so the server sends nothing.
		async setup(page) {
			await page.route(
				(u) => u.pathname === '/api/settings',
				async (route) => {
					if (route.request().method() !== 'GET') return route.continue();
					const response = await route.fetch();
					const body = await response.json();
					body.sending = { ...body.sending, enabled: true, has_password: true };
					await route.fulfill({ response, json: body });
				}
			);
		}
	},
	{
		name: 'client-report',
		path: 'settings/client-reports',
		async before(page) {
			const row = page.locator('li', { hasText: org.name });
			await row.getByRole('button', { name: 'Preview', exact: true }).click();
			await page.getByRole('dialog').getByText('DMARC report for').first().waitFor();
		}
	},
	{
		name: 'setup',
		root: setupBase,
		path: 'setup',
		async before(page) {
			await page.getByLabel('Organization name').fill('Example IT');
			await page.getByRole('button', { name: 'Continue' }).click();
			await page.getByText('Connect the report mailbox').waitFor();
			await page.getByLabel('Host').fill('imap.example-it.example');
			await page.getByLabel('Username').fill('dmarc-reports@example-it.example');
			await page.getByLabel('Password', { exact: true }).fill('not-a-password');
		}
	}
];

/** Waits until the page is idle: no request in flight, no skeleton, charts drawn. */
async function settle(page) {
	await page.waitForLoadState('networkidle');
	await page
		.waitForFunction(() => !document.querySelector('[data-slot="skeleton"], .animate-pulse'), null, {
			timeout: 15000
		})
		.catch(() => console.warn('  a skeleton is still showing'));
	await page.evaluate(() => document.fonts.ready);
	await page.waitForTimeout(800);
}

await mkdir(outDir, { recursive: true });
const browser = await chromium.launch();
for (const scheme of ['light', 'dark']) {
	const context = await browser.newContext({
		viewport: { width: 1440, height: 900 },
		deviceScaleFactor: 2,
		locale: 'en-GB',
		timezoneId: 'Europe/Zurich',
		colorScheme: scheme,
		reducedMotion: 'reduce'
	});
	for (const shot of SHOTS) {
		if (only && !only.has(shot.name)) continue;
		const root = 'root' in shot ? shot.root : base;
		if (!root) {
			console.log(`skip ${shot.name}: no setup base URL`);
			continue;
		}
		const page = await context.newPage();
		if (shot.setup) await shot.setup(page);
		await page.goto(url(root, shot.path));
		await settle(page);
		if (shot.before) {
			await shot.before(page);
			await settle(page);
		}
		// No hover or focus ring in the picture.
		await page.mouse.move(0, 0);
		await page.evaluate(() => document.activeElement?.blur());
		const file = join(outDir, `${shot.name}-${scheme}.png`);
		await page.screenshot({ path: file });
		console.log(file);
		await page.close();
	}
	await context.close();
}
await browser.close();
