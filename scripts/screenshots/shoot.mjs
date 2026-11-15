// Takes the docs screenshots (docs/public/screenshots/<name>-light.png and -dark.png)
// from a running Marcan: 1440x900 at 2x, en-GB, light and dark from prefers-color-scheme.
//
//   node shoot.mjs <base-url> <out-dir> [<setup-base-url>] [--only name,name]
//
// <setup-base-url> is a second server on an empty data directory, for the setup's Accounts
// step (its Organization step is confirmed as "Example IT" on the way); without it the
// setup shot is skipped. On the demo server nothing is saved or sent: forms are filled in
// the page only, and the account shots show the accounts as a configured installation has
// them (`configured`) in the page's answers only.
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

const now = Math.floor(Date.now() / 1000);
const DAY = 86400;

/**
 * The demo's accounts as a running installation has them, in the page's answers only: the demo
 * stores two (Reports receives, Notifications sends) with placeholder passwords, whose checks fail
 * on the made-up host. Added here: a client's Microsoft 365 account (a real one would ask
 * Microsoft for a token; left out with `microsoft: false`, for the form that adds it), the
 * receiving accounts checked minutes ago, and sending switched on.
 */
async function configured(page, { microsoft: withMicrosoft = true } = {}) {
	await page.route(
		(u) => u.pathname === '/api/settings' || u.pathname === '/api/system/health',
		async (route) => {
			if (route.request().method() !== 'GET') return route.continue();
			const response = await route.fetch();
			const body = await response.json();
			if (new URL(route.request().url()).pathname === '/api/settings') {
				const [reports] = body.accounts;
				const microsoft = {
					...reports,
					id: 3,
					name: 'Nordlicht Logistik',
					provider: 'microsoft365',
					address: 'dmarc@nordlicht-group.example',
					login: {
						kind: 'microsoft',
						tenant_id: '3f2b8c1e-7a4d-4e9b-9c2a-5d1e8f6a0b47',
						client_id: '8d41a7c2-1e5f-4b3a-a9d6-2c7e0f9b5a13',
						certificate: { thumbprint: '4C7A1E9B2D58F03A6E1C9B7D2F4A8E05C3B6D190', expires_at: now + 700 * DAY }
					},
					// A Microsoft login has no servers: it reads and sends over Microsoft Graph.
					imap: null,
					smtp: null,
					receiving: { enabled: true, folder: 'DMARC', forwarded_from: [] },
					ready_to_receive: true,
					report_address: true,
					sends: true,
					ready_to_send: true,
					credential: { ...reports.credential, expires_at: now + 700 * DAY, expiry_text: null },
					shared_mailbox: null
				};
				if (withMicrosoft) body.accounts = [...body.accounts, microsoft];
				body.sending = { ...body.sending, enabled: true };
			} else {
				const health = (a, ago) => ({
					id: a.id,
					name: a.name,
					address: a.address,
					state: 'active',
					status: 'pass',
					last_check_at: now - ago,
					last_success_at: now - ago,
					last_error: null,
					quarantine_count: 0
				});
				body.accounts = [
					health({ id: 1, name: 'Reports', address: 'dmarc-reports@example-it.example' }, 140),
					...(withMicrosoft
						? [health({ id: 3, name: 'Nordlicht Logistik', address: 'dmarc@nordlicht-group.example' }, 70)]
						: [])
				];
				body.receive_configured = true;
				body.last_check_at = now - 140;
				body.receive_last_success_at = now - 140;
				body.last_poll = { at: now - 140, failed: false };
				// The demo's own checks fail on the made-up host; with them passing, so does the whole.
				body.pipeline = { ...body.pipeline, mailbox: 'active', mailbox_status: 'pass', status: 'pass', headline: 'all_well' };
			}
			await route.fulfill({ response, json: body });
		}
	);
}

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
	{ name: 'accounts', path: 'settings/accounts', setup: configured },
	{
		name: 'account',
		path: 'settings/accounts/new',
		// Tall enough for the whole form with the test's lines and the save bar under it.
		height: 1320,
		// A new Microsoft 365 account with a generated certificate (kept in the server's memory
		// for a day, never saved) and a passing test, answered in the page: a real test would
		// ask Microsoft for a token.
		async setup(page) {
			await configured(page, { microsoft: false });
			await page.route(
				(u) => u.pathname === '/api/accounts/test',
				(route, request) =>
					route.fulfill({
						json: {
							token: {
								ok: true,
								message: 'Microsoft issued a token for the app registration.',
								detail: null
							},
							mailbox: {
								ok: true,
								message: 'Read dmarc@nordlicht-group.example over Microsoft Graph.',
								detail: null,
								folders: ['Archive', 'DMARC', 'Deleted Items', 'Drafts', 'INBOX', 'Junk Email', 'Sent Items'].map(
									(name) => ({ name, delimiter: '/' })
								),
								selected: { name: request.postDataJSON().folder || 'INBOX', exists: true, messages: 412 }
							},
							// Microsoft Graph has no sending step: the test sends nothing.
							smtp: null
						}
					})
			);
		},
		async before(page) {
			await page.getByRole('radio', { name: /^Microsoft 365/ }).check();
			await page.getByLabel('Name', { exact: true }).fill('Nordlicht Logistik');
			await page.getByLabel('Email address').fill('dmarc@nordlicht-group.example');
			await page.getByLabel('Tenant ID').fill('3f2b8c1e-7a4d-4e9b-9c2a-5d1e8f6a0b47');
			await page.getByLabel('Client ID').fill('8d41a7c2-1e5f-4b3a-a9d6-2c7e0f9b5a13');
			await page.getByRole('button', { name: 'Generate', exact: true }).click();
			await page.getByTestId('certificate-thumbprint').waitFor({ timeout: 60000 });
			await page.getByRole('button', { name: 'Test', exact: true }).click();
			await page.getByText('folders listed').waitFor();
			await page.getByLabel('Folder').selectOption('DMARC');
		}
	},
	{ name: 'sending', path: 'settings/sending', setup: configured },
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
			await page.getByRole('heading', { name: 'The accounts reports arrive in' }).waitFor();
			await page.getByRole('radio', { name: /^IMAP\/SMTP/ }).check();
			await page.getByLabel('Name', { exact: true }).fill('Reports');
			await page.getByLabel('Email address').fill('dmarc-reports@example-it.example');
			await page.getByLabel('Password', { exact: true }).fill('not-a-password');
			// Advanced is open while a provider without presets has no server, and folds once one is in.
			await page.getByLabel('Host').first().fill('imap.example-it.example');
			await page.evaluate(() => window.scrollTo(0, 0));
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
		if (shot.height) await page.setViewportSize({ width: 1440, height: shot.height });
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
