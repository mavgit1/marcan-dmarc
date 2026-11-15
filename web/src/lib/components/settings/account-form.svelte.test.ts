// @vitest-environment happy-dom
// The account form as it renders: a new account's provider chosen first and a stored one's
// fixed, its rows in their order, the username that follows the address, Advanced folded until it has to be filled
// in and open from then on (none at all for Microsoft 365), the stored password that applies only
// to its own servers, a Microsoft login's certificate, a disabled Save that says why, the last check
// of a failing account, and the note that credentials are not encrypted without MARCAN_SECRET_KEY.
import { fireEvent } from '@testing-library/svelte';
import { beforeEach, describe, expect, it, vi } from 'vitest';
import { formatDay } from '$lib/format';
import { owned, pendingFetch, renderPage, saveBar, settle } from '$lib/test/component-test.svelte';
import { accountFixture, CONNECTED, settingsFixture } from '$lib/test/settings-fixture';
import { meta } from '$lib/state/meta.svelte';
import { systemHealth } from '$lib/state/system-health.svelte';
import type { ProviderOption } from '$lib/types/ProviderOption';
import type { SystemHealthResponse } from '$lib/types/SystemHealthResponse';
import type { AccountTestResponse } from '$lib/types/AccountTestResponse';
import type { CertificateDraft } from '$lib/types/CertificateDraft';
import type { Meta } from '$lib/types/Meta';
import type { SettingsResponse } from '$lib/types/SettingsResponse';
import { ACCOUNT_TEXT, AccountModel, MS_TEXT, toRequest } from './account-draft.svelte';

import AccountForm from './account-form.svelte';

const TENANT = '11111111-2222-3333-4444-555555555555';
const CLIENT = 'aaaaaaaa-bbbb-cccc-dddd-eeeeeeeeeeee';

/** A stored Microsoft 365 account with its certificate. */
const MICROSOFT = accountFixture({
	provider: 'microsoft365',
	login: {
		kind: 'microsoft',
		tenant_id: TENANT,
		client_id: CLIENT,
		certificate: { thumbprint: 'FFEE', expires_at: 1_856_304_000 }
	},
	imap: null,
	smtp: null
});

beforeEach(pendingFetch);

async function page(saved: SettingsResponse, id: number | null) {
	const model = owned(() => new AccountModel({ source: { value: () => saved }, id }));
	const m = model.value;
	const view = await renderPage(AccountForm, { model: m }, { page: 'accounts' });
	const field = <E extends HTMLElement = HTMLInputElement>(name: string) =>
		view.container.querySelector<E>(`#account-${name}`)!;
	/** Type `value` into the field, then leave it. */
	const type = async (name: string, value: string) => {
		await fireEvent.input(field(name), { target: { value } });
		await fireEvent.change(field(name));
		await fireEvent.focusOut(field(name));
		await settle();
	};
	/** Pick a new account's provider. */
	const pick = async (provider: ProviderOption) => {
		await fireEvent.click(field(`provider-${provider}`));
		await settle();
	};
	return { m, view, field, type, pick, stop: model.stop };
}

describe('AccountForm', () => {
	it('shows its rows in order, Advanced folded, and no save bar while nothing changed', async () => {
		const { view, field, stop } = await page(settingsFixture(CONNECTED), 1);
		const titles = [...view.container.querySelectorAll('h3')].map((h) => h.textContent?.trim());
		expect(titles).toEqual([
			'Provider',
			'Name',
			'Email address',
			'Login',
			'Receive reports',
			'Advanced',
			'IMAP server',
			'SMTP server',
			'Test'
		]);
		// A stored account's provider is a plain line, never a choice.
		expect(view.getByTestId('account-provider').textContent).toBe('IMAP/SMTP');
		expect(view.queryByRole('radiogroup', { name: 'Provider' })).toBeNull();
		expect(field('name').value).toBe('Reports');
		expect(field('imap-host').closest('[id="account-advanced"]')?.className).toContain('hidden');
		expect(view.getByRole('button', { expanded: false }).textContent).toContain('imap.example.com:993 (TLS)');
		// The stored password is a locked field.
		expect(field('password').value).toBe('Stored');
		expect(view.queryByRole('button', { name: 'Save' })).toBeNull();
		stop();
	});

	it('opens a new account with the provider choice alone, nothing picked', async () => {
		const { m, view, field, pick, stop } = await page(settingsFixture(), null);
		const radios = view.getAllByRole('radio') as HTMLInputElement[];
		expect(radios.map((r) => r.closest('label')?.querySelector('.font-medium')?.textContent)).toEqual([
			'IMAP/SMTP',
			'Google',
			'Microsoft 365'
		]);
		expect(radios.some((r) => r.checked)).toBe(false);
		expect(view.getAllByRole('link', { name: 'docs' })).toHaveLength(3);
		expect(field('name')).toBeNull();
		expect(view.queryByRole('button', { name: 'Test' })).toBeNull();
		expect(m.providerChosen).toBe(false);
		await pick('google');
		expect(field<HTMLInputElement>('provider-google').checked).toBe(true);
		expect(field('name')).toBeTruthy();
		// The choice stays, and can still be changed, until the save.
		await pick('generic');
		expect(m.draft.imapHost).toBe('');
		expect(view.getAllByRole('radio')).toHaveLength(3);
		stop();
	});

	it('marks no field of a new account before a field is left', async () => {
		const { m, view, field, type, pick, stop } = await page(settingsFixture(), null);
		await pick('generic');
		// Save waits for the address, but the empty form is not an error yet.
		expect(m.invalid?.field).toBe('address');
		expect(m.held).toBeNull();
		expect(field('address').getAttribute('aria-invalid')).toBeNull();
		// Filled in without leaving a field (a browser's autofill): still nothing said, no save bar.
		await fireEvent.input(field('username'), { target: { value: 'saved@example.com' } });
		await fireEvent.input(field('password'), { target: { value: 'saved' } });
		await settle();
		expect(m.dirty).toBe(false);
		expect(view.container.querySelector('[aria-invalid="true"]')).toBeNull();
		expect(view.queryByText('Enter the email address reports are sent to.')).toBeNull();
		expect(view.queryByRole('button', { name: 'Save' })).toBeNull();
		// Left: the problem is said under the field and beside the disabled Save.
		await type('address', 'reports');
		expect(field('address').getAttribute('aria-invalid')).toBe('true');
		const { status, save } = saveBar(view.container);
		expect(save.disabled).toBe(true);
		expect(status?.textContent?.trim()).toBe('The email address reports is not a valid email address.');
		stop();
	});

	it('says a field is missing only once it was typed in and left, or Save was pressed', async () => {
		const { view, field, type, pick, stop } = await page(settingsFixture(), null);
		await pick('google');
		await type('name', 'Reports');
		// The address is missing, but the user has not come to it yet: Save is there, enabled.
		expect(view.queryByText('Enter the email address reports are sent to.')).toBeNull();
		expect(view.container.querySelector('[aria-invalid="true"]')).toBeNull();
		expect(saveBar(view.container).status?.textContent?.trim()).toBe('Unsaved changes');
		expect(saveBar(view.container).save.disabled).toBe(false);
		// Save pressed: nothing is saved, the problem is said under its field and beside Save.
		await fireEvent.submit(view.container.querySelector('form')!);
		await settle();
		expect(field('address').getAttribute('aria-invalid')).toBe('true');
		expect(view.getAllByText('Enter the email address reports are sent to.')).toHaveLength(2);
		expect(saveBar(view.container).save.disabled).toBe(true);
		// Filled in: the next missing field is said at once now.
		await type('address', 'dmarc@example.org');
		expect(saveBar(view.container).status?.textContent?.trim()).toBe(ACCOUNT_TEXT.password);
		stop();
	});

	it('says nothing after a provider that fills in its servers was picked and left', async () => {
		for (const provider of ['google', 'microsoft365', 'generic'] as const) {
			const { m, view, field, pick, stop } = await page(settingsFixture(), null);
			await pick(provider);
			// Focus moves on from the choice: the servers it filled in are not the user's edits.
			await fireEvent.focusOut(field(`provider-${provider}`));
			await settle();
			expect(m.dirty).toBe(false);
			expect(view.queryByRole('button', { name: 'Save' })).toBeNull();
			expect(view.queryByText('Enter the email address reports are sent to.')).toBeNull();
			stop();
		}
	});

	it('starts IMAP/SMTP without servers and Advanced open', async () => {
		const { m, view, field, pick, stop } = await page(settingsFixture(), null);
		await pick('generic');
		expect(m.draft.provider).toBe('generic');
		expect(m.draft.imapHost).toBe('');
		expect(m.draft.loginKind).toBe('password');
		expect(view.getAllByRole('link', { name: 'docs' })[0].getAttribute('href')).toMatch(/accounts#imap-smtp$/);
		expect(field('imap-host').closest('[id="account-advanced"]')?.className).not.toContain('hidden');
		// Format hints, never values.
		expect(field('imap-host').placeholder).toBe('imap.provider.tld');
		expect(field('address').placeholder).toBe('name@domain');
		stop();
	});

	it('fills in Google with a password login, and Microsoft 365 with its login only', async () => {
		const { m, view, field, pick, stop } = await page(settingsFixture(), null);
		await pick('google');
		expect(m.draft).toMatchObject({
			imapHost: 'imap.gmail.com',
			smtpHost: 'smtp.gmail.com',
			smtpPort: 465,
			loginKind: 'password'
		});
		expect(view.getByText(/app password/)).toBeTruthy();
		expect(field('password')).toBeTruthy();

		await pick('microsoft365');
		expect(m.draft.loginKind).toBe('microsoft');
		expect(field('tenant')).toBeTruthy();
		// Its hint points to the guide that sets up the mailbox.
		expect(view.getByText(/set up the mailbox with the guide/)).toBeTruthy();
		// The provider decides the login: no choice of it.
		expect(view.queryByRole('radiogroup', { name: 'Login' })).toBeNull();
		expect(view.queryByRole('radio', { name: 'Password' })).toBeNull();
		// An empty ID is asked for; its format is said only once something is typed.
		expect(view.getByText(MS_TEXT.enterTenant)).toBeTruthy();
		expect(view.queryByText(MS_TEXT.tenant)).toBeNull();

		// Back to IMAP/SMTP: Google's servers go, a password login again.
		await pick('generic');
		expect(m.draft).toMatchObject({ imapHost: '', smtpHost: '', loginKind: 'password' });
		stop();
	});

	it('shows a stored generic account on Gmail as Google', async () => {
		const gmail = accountFixture({
			provider: 'google',
			imap: { host: 'imap.gmail.com', port: 993, security: 'tls' },
			smtp: { host: 'smtp.gmail.com', port: 465, security: 'tls' }
		});
		const { view, stop } = await page(settingsFixture(CONNECTED, { accounts: [gmail] }), 1);
		expect(view.getByTestId('account-provider').textContent).toBe('Google');
		stop();
	});

	it('keeps Advanced open while its host is typed, until its toggle folds it', async () => {
		const { m, view, field, type, pick, stop } = await page(settingsFixture(), null);
		await pick('generic');
		const advanced = () => field('imap-host').closest('[id="account-advanced"]')!.className;
		expect(advanced()).not.toContain('hidden');
		// The first character fills a host: Advanced stays open, so the rest of the typing lands.
		await type('imap-host', 's');
		expect(advanced()).not.toContain('hidden');
		await type('smtp-host', 's');
		await type('smtp-host', 'smtp.example.org');
		expect(advanced()).not.toContain('hidden');
		expect(m.draft.smtpHost).toBe('smtp.example.org');
		await fireEvent.click(view.getByRole('button', { expanded: true }));
		await settle();
		expect(advanced()).toContain('hidden');
		stop();
	});

	it('holds an account with neither server, receiving off', async () => {
		const { view, field, type, stop } = await page(settingsFixture(CONNECTED), 1);
		await fireEvent.click(field('receive'));
		await type('imap-host', '');
		await type('smtp-host', '');
		const { status, save } = saveBar(view.container);
		expect(save.disabled).toBe(true);
		expect(status?.textContent?.trim()).toBe(ACCOUNT_TEXT.noServer);
		expect(field('imap-host').getAttribute('aria-invalid')).toBe('true');
		stop();
	});

	it('fills the username from the address until it is typed', async () => {
		const { m, type, pick, stop } = await page(settingsFixture(), null);
		await pick('generic');
		await type('address', 'dmarc@example.org');
		expect(m.draft.username).toBe('dmarc@example.org');
		await type('username', 'login-name');
		await type('address', 'reports@example.org');
		expect(m.draft.username).toBe('login-name');
		stop();
	});

	it('asks for the password again once the server or username changes', async () => {
		const { m, view, field, type, stop } = await page(settingsFixture(CONNECTED), 1);
		await type('imap-host', 'imap.other.example');
		expect(m.passwordStored).toBe(false);
		expect(field('password').value).toBe('');
		// One sentence under the field: the specific one, worded like the server's 409.
		expect(view.getAllByText(ACCOUNT_TEXT.passwordAgain, { selector: 'p' })).toHaveLength(1);
		expect(view.queryByText(ACCOUNT_TEXT.password, { selector: 'p' })).toBeNull();
		const { status, save } = saveBar(view.container);
		expect(save.disabled).toBe(true);
		expect(status?.textContent?.trim()).toBe(ACCOUNT_TEXT.passwordAgain);
		expect(field('password').getAttribute('aria-invalid')).toBe('true');

		await type('password', 'secret');
		expect(saveBar(view.container).save.disabled).toBe(false);
		// Back to the saved server: the stored password applies again.
		await type('imap-host', 'imap.example.com');
		await type('password', '');
		expect(m.passwordStored).toBe(true);
		stop();
	});

	it('keeps the stored password when a port is emptied: the server fills in the same one', async () => {
		const { m, field, type, stop } = await page(settingsFixture(CONNECTED), 1);
		await type('imap-port', '');
		await type('smtp-port', '');
		expect(m.passwordStored).toBe(true);
		expect(field('password').value).toBe('Stored');
		stop();
	});

	it('shows a Microsoft 365 account without Advanced or any server field', async () => {
		const { view, field, stop } = await page(settingsFixture(CONNECTED, { accounts: [MICROSOFT] }), 1);
		const titles = [...view.container.querySelectorAll('h3')].map((h) => h.textContent?.trim());
		expect(titles).toEqual(['Provider', 'Name', 'Email address', 'Login', 'Receive reports', 'Test']);
		for (const name of ['imap-host', 'imap-port', 'imap-security', 'smtp-host', 'smtp-port', 'smtp-security'])
			expect(field(name)).toBeNull();
		// No credential to choose, no secret, no expiry date to enter.
		expect(view.queryByRole('radiogroup', { name: 'Credential' })).toBeNull();
		expect(view.container.querySelector('input[type="date"]')).toBeNull();
		expect(view.container.querySelector('input[type="password"]')).toBeNull();
		expect(view.queryByText(/secret/i)).toBeNull();
		stop();
		view.unmount();

		// A new one the same, from the moment it is picked.
		const fresh = await page(settingsFixture(), null);
		await fresh.pick('microsoft365');
		expect(fresh.view.queryByText('Advanced')).toBeNull();
		expect(fresh.field('imap-host')).toBeNull();
		expect(fresh.field('smtp-host')).toBeNull();
		fresh.stop();
	});

	it('saves a rename of an account whose certificate expired', async () => {
		const expired = accountFixture({
			...MICROSOFT,
			login: {
				kind: 'microsoft',
				tenant_id: TENANT,
				client_id: CLIENT,
				certificate: { thumbprint: 'FFEE', expires_at: Date.parse('2020-01-01T23:59:59Z') / 1000 }
			}
		});
		const { view, type, stop } = await page(settingsFixture(CONNECTED, { accounts: [expired] }), 1);
		await type('name', 'Renamed');
		expect(saveBar(view.container).save.disabled).toBe(false);
		stop();
	});

	it('receiving without an address: Save says why and marks the field', async () => {
		const { view, field, type, stop } = await page(settingsFixture(CONNECTED), 1);
		await type('address', '');
		const { status, save } = saveBar(view.container);
		expect(save.disabled).toBe(true);
		expect(status?.textContent?.trim()).toBe('Enter the email address reports are sent to.');
		expect(field('address').getAttribute('aria-invalid')).toBe('true');
		stop();
	});

	it('adds forwarded addresses row by row, removes them, and shows saved ones at once', async () => {
		const first = await page(settingsFixture(CONNECTED), 1);
		expect(first.view.container.querySelector('#account-forwarded-0')).toBeNull();
		await fireEvent.click(first.view.getByRole('button', { name: 'Reports forwarded from other addresses?' }));
		await settle();
		expect(first.field('forwarded-0').placeholder).toBe('name@domain');
		// An empty row is no change.
		expect(first.m.dirty).toBe(false);
		await first.type('forwarded-0', 'dmarc@client1.example');
		await fireEvent.click(first.view.getByRole('button', { name: 'Add another address' }));
		await settle();
		await first.type('forwarded-1', 'DMARC@client1.example');
		expect(first.field('forwarded-1').getAttribute('aria-invalid')).toBe('true');
		expect(first.m.held?.message).toBe('The forwarding address DMARC@client1.example is entered twice.');
		await first.type('forwarded-1', 'dmarc@client2.example');
		expect(first.m.held).toBeNull();
		expect(first.m.normalized.forwardedFrom).toEqual(['dmarc@client1.example', 'dmarc@client2.example']);
		await fireEvent.click(first.view.getByRole('button', { name: 'Remove dmarc@client1.example' }));
		await settle();
		expect(first.m.draft.forwardedFrom).toEqual(['dmarc@client2.example']);
		expect(first.field('forwarded-0').value).toBe('dmarc@client2.example');
		first.stop();

		const forwarded = settingsFixture(CONNECTED, {
			accounts: [
				accountFixture({
					receiving: { enabled: true, folder: 'INBOX', forwarded_from: ['old@example.net', 'dmarc@client.example'] }
				})
			]
		});
		const second = await page(forwarded, 1);
		expect(second.field('forwarded-0').value).toBe('old@example.net');
		expect(second.field('forwarded-1').value).toBe('dmarc@client.example');
		second.stop();
	});

	it('holds a Microsoft login until its IDs are GUIDs and its certificate is there', async () => {
		const { m, view, type, field, stop } = await page(settingsFixture(), null);
		m.setProvider('microsoft365');
		await settle();
		await type('address', 'dmarc@example.com');
		const held = () => saveBar(view.container).status?.textContent?.trim();
		await type('tenant', '');
		expect(held()).toBe(MS_TEXT.enterTenant);
		await type('tenant', 'contoso.onmicrosoft.com');
		expect(held()).toBe(MS_TEXT.tenant);
		expect(field('tenant').getAttribute('aria-invalid')).toBe('true');
		await type('tenant', TENANT);
		// The client ID was not typed in yet: nothing is said about it until Save is pressed.
		expect(held()).toBe('Unsaved changes');
		expect(field('client').getAttribute('aria-invalid')).toBeNull();
		expect(saveBar(view.container).save.disabled).toBe(false);
		await fireEvent.submit(view.container.querySelector('form')!);
		await settle();
		expect(held()).toBe(MS_TEXT.enterClient);
		expect(field('client').getAttribute('aria-invalid')).toBe('true');
		expect(saveBar(view.container).save.disabled).toBe(true);
		await type('client', 'Marcan');
		expect(held()).toBe(MS_TEXT.client);
		await type('client', CLIENT);
		// The certificate has no field to type in: said at once.
		expect(held()).toBe(MS_TEXT.certificate);
		expect(view.getAllByText(MS_TEXT.certificate, { selector: 'p' })).toHaveLength(1);
		expect((view.getByRole('button', { name: 'Test' }) as HTMLButtonElement).disabled).toBe(true);
		expect(m.draft.loginKind).toBe('microsoft');
		stop();
	});

	it('generates a certificate for the form, shows it, and saves it untested with nothing else', async () => {
		const draft: CertificateDraft = {
			draft_id: 'd1',
			thumbprint: 'AB12CD34',
			expires_at: 1_856_304_000,
			subject: 'dmarc@example.com'
		};
		const asked: unknown[] = [];
		vi.stubGlobal(
			'fetch',
			vi.fn(async (input: Request | string) => {
				const request = typeof input === 'string' ? new Request(input) : input;
				if (request.url.endsWith('/api/accounts/certificate')) {
					asked.push(await request.clone().json());
					return new Response(JSON.stringify(draft), { headers: { 'content-type': 'application/json' } });
				}
				return new Promise<Response>(() => {});
			})
		);
		const { m, view, type, stop } = await page(settingsFixture(), null);
		m.setProvider('microsoft365');
		await settle();
		await type('address', 'dmarc@example.com');
		await type('tenant', TENANT);
		await type('client', CLIENT);
		expect(saveBar(view.container).status?.textContent?.trim()).toBe(MS_TEXT.certificate);
		await fireEvent.click(view.getByRole('button', { name: 'Generate' }));
		await settle();
		await settle();
		expect(asked).toEqual([{ subject: 'dmarc@example.com' }]);
		expect(view.getByTestId('certificate-thumbprint').textContent).toBe('AB12CD34');
		expect(view.getByText(formatDay(draft.expires_at))).toBeTruthy();
		const download = view.getByRole('link', { name: 'Download .cer' });
		expect(download.getAttribute('href')).toBe('/api/accounts/certificate/d1');
		expect(view.getByRole('button', { name: 'Generate new' })).toBeTruthy();
		expect(toRequest(m.normalized)).toMatchObject({
			login: { kind: 'microsoft', tenant_id: TENANT, client_id: CLIENT, draft_id: 'd1' },
			imap: null,
			smtp: null
		});
		expect(view.getByText(/^Save, then upload the \.cer to the app registration/)).toBeTruthy();
		expect(saveBar(view.container).save.disabled).toBe(false);
		// The save stores it without a test: the app registration knows it only once uploaded.
		await fireEvent.click(saveBar(view.container).save);
		await settle();
		expect(m.testing).toBe(false);
		expect(m.result).toBeNull();
		stop();
	});

	it('shows a stored certificate with its download, and keeps it for other IDs', async () => {
		const { m, view, type, stop } = await page(settingsFixture(CONNECTED, { accounts: [MICROSOFT] }), 1);
		expect(view.getByTestId('certificate-thumbprint').textContent).toBe('FFEE');
		expect(view.getByText(formatDay(1_856_304_000))).toBeTruthy();
		expect(view.getByRole('link', { name: 'Download .cer' }).getAttribute('href')).toBe(
			'/api/accounts/1/certificate.cer'
		);
		expect(view.getByRole('button', { name: 'Generate new' })).toBeTruthy();
		expect(view.queryByRole('button', { name: 'Save' })).toBeNull();
		// Another app registration: the certificate stays the account's.
		await type('client', 'bbbbbbbb-bbbb-cccc-dddd-eeeeeeeeeeee');
		expect(m.microsoftProblem).toBeNull();
		expect(saveBar(view.container).save.disabled).toBe(false);
		stop();
	});

	it('holds the test until there is a server and a password', async () => {
		const { m, view, stop } = await page(settingsFixture(), null);
		m.setProvider('microsoft365');
		await settle();
		const test = view.getByRole('button', { name: 'Test' }) as HTMLButtonElement;
		// Microsoft 365 needs no server, but its login needs the tenant ID first.
		expect(test.disabled).toBe(true);
		expect(view.getByText(MS_TEXT.enterTenant)).toBeTruthy();
		stop();
		view.unmount();
		const generic = await page(settingsFixture(), null);
		await generic.pick('generic');
		expect((generic.view.getByRole('button', { name: 'Test' }) as HTMLButtonElement).disabled).toBe(true);
		expect(generic.view.getByText(ACCOUNT_TEXT.serverToTest)).toBeTruthy();
		generic.stop();
	});

	it('says once under the login that credentials are not encrypted without a key', async () => {
		const NOTE = 'Not encrypted';
		meta.current = { secrets_encrypted: false } as Meta;
		const plain = await page(settingsFixture(CONNECTED), 1);
		expect(plain.view.container.textContent?.split(NOTE).length).toBe(2);
		plain.stop();
		plain.view.unmount();

		// A new account says it only once a credential is entered.
		const fresh = await page(settingsFixture(), null);
		await fresh.pick('generic');
		expect(fresh.view.container.textContent).not.toContain(NOTE);
		await fresh.type('password', 'secret');
		expect(fresh.view.container.textContent?.split(NOTE).length).toBe(2);
		fresh.stop();
		fresh.view.unmount();

		meta.current = { secrets_encrypted: true } as Meta;
		const sealed = await page(settingsFixture(CONNECTED), 1);
		expect(sealed.view.container.textContent).not.toContain(NOTE);
		sealed.stop();
		meta.current = null;
	});

	it("shows a failing account's last check and its reason under Test", async () => {
		const reason = 'The host imap.example.com does not resolve.';
		systemHealth.current = {
			accounts: [
				{
					id: 1,
					name: 'Reports',
					address: 'reports@example.com',
					state: 'error',
					status: 'fail',
					last_check_at: Math.floor(Date.now() / 1000) - 300,
					last_success_at: null,
					last_error: reason,
					last_error_detail: 'getaddrinfo: nodename nor servname provided',
					quarantine_count: 0
				}
			]
		} as unknown as SystemHealthResponse;
		const { view, stop } = await page(settingsFixture(CONNECTED), 1);
		const box = view.getByTestId('account-last-check');
		const line = box.textContent ?? '';
		expect(line).toMatch(/^\s*Error, checked /);
		expect(line).toContain(reason);
		// The server's answer is folded under Details, after the sentence.
		const details = box.querySelector('details');
		expect(details?.querySelector('summary')?.textContent).toBe('Details');
		expect(details?.textContent).toContain('getaddrinfo: nodename nor servname provided');
		stop();
		systemHealth.current = null;
	});

	it('says a failure once: a test line that says it leaves the last check its state word', async () => {
		const sentence = 'Login rejected. Check the username and password.';
		const other = 'The host imap.example.com does not resolve.';
		const health = (last_error: string) =>
			({
				accounts: [
					{
						id: 1,
						name: 'Reports',
						address: 'reports@example.com',
						state: 'error',
						status: 'fail',
						last_check_at: Math.floor(Date.now() / 1000) - 300,
						last_success_at: null,
						last_error,
						quarantine_count: 0
					}
				]
			}) as unknown as SystemHealthResponse;
		const answer: AccountTestResponse = {
			token: null,
			mailbox: {
				ok: false,
				message: sentence,
				detail: 'NO [AUTHENTICATIONFAILED]',
				docs: null,
				folders: [],
				selected: null
			},
			smtp: null
		};
		vi.stubGlobal(
			'fetch',
			vi.fn(async (input: Request | string) => {
				const request = typeof input === 'string' ? new Request(input) : input;
				if (request.url.endsWith('/api/accounts/test')) {
					return new Response(JSON.stringify(answer), { headers: { 'content-type': 'application/json' } });
				}
				return new Promise<Response>(() => {});
			})
		);
		systemHealth.current = health(`${sentence} (NO [AUTHENTICATIONFAILED])`);
		const { m, view, stop } = await page(settingsFixture(CONNECTED), 1);
		const status = () => view.getByTestId('account-last-check').textContent ?? '';
		expect(status()).toContain(sentence);
		await m.testLogin();
		await settle();
		expect(status()).toMatch(/^\s*Error, checked [^.]*$/);
		expect(view.container.textContent?.split(sentence).length).toBe(2);
		// Another stored error than the test's stays: it is not said below.
		systemHealth.current = health(other);
		await settle();
		expect(status()).toContain(other);
		stop();
		systemHealth.current = null;
	});
});
