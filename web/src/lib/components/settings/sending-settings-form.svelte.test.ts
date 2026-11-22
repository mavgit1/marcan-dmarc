// @vitest-environment happy-dom
// The Sending page as it renders: the account select over the accounts that can send, the
// "Sending as" line with the chosen account's address (no field: mail always comes from it), the
// rows disabled while the switch is off, and a disabled Save that says why and marks the select.
import { fireEvent } from '@testing-library/svelte';
import { beforeEach, describe, expect, it } from 'vitest';
import { owned, pendingFetch, renderPage, saveBar, settle } from '#lib/test/component-test.svelte.ts';
import { accountFixture, CONNECTED, SENDING_ON, settingsFixture } from '#lib/test/settings-fixture.ts';
import type { SettingsResponse } from '#lib/types/SettingsResponse.ts';
import { SendingModel } from './sending-draft.svelte';
import SendingSettingsForm from './sending-settings-form.svelte';

beforeEach(pendingFetch);

async function page(saved: SettingsResponse) {
	const model = owned(() => new SendingModel({ source: { value: () => saved } }));
	const m = model.value;
	const view = await renderPage(SendingSettingsForm, { model: m }, { feature: () => m.feature });
	return {
		m,
		view,
		stop: model.stop,
		input: (id: string) => view.container.querySelector<HTMLInputElement>(`#${id}`)!,
		select: () => view.container.querySelector<HTMLSelectElement>('#smtp-account')!
	};
}

/** Three accounts that can send (one a Microsoft login, which sends over Graph) and one that only receives. */
const THREE = settingsFixture(CONNECTED, {
	accounts: [
		accountFixture(),
		accountFixture({
			id: 2,
			name: 'Mailer',
			address: 'mailer@example.net',
			smtp: { host: 'smtp.example.net', port: 587, security: 'starttls' }
		}),
		accountFixture({
			id: 3,
			name: 'Inbox only',
			address: 'inbox@example.org',
			smtp: { host: '', port: null, security: 'tls' },
			sends: false,
			ready_to_send: false
		}),
		accountFixture({
			id: 4,
			name: 'Microsoft',
			address: 'dmarc@contoso.example',
			provider: 'microsoft365',
			login: {
				kind: 'microsoft',
				tenant_id: '11111111-2222-3333-4444-555555555555',
				client_id: 'aaaaaaaa-bbbb-cccc-dddd-eeeeeeeeeeee',
				certificate: { thumbprint: 'AB', expires_at: null }
			},
			imap: null,
			smtp: null
		})
	]
});

describe('SendingSettingsForm', () => {
	it('offers the accounts that can send, each by its name and its way out', async () => {
		const { select, stop, view } = await page(THREE);
		const options = [...select().options].map((o) => o.textContent?.trim());
		expect(options).toEqual([
			'None chosen',
			'Reports (smtp.example.com)',
			'Mailer (smtp.example.net)',
			'Microsoft (Microsoft Graph)'
		]);
		expect(view.queryByRole('button', { name: 'Save' })).toBeNull();
		stop();
	});

	it('without an account that can send, says where to add one', async () => {
		const { view, stop } = await page(
			settingsFixture({
				accounts: [
					accountFixture({ smtp: { host: '', port: null, security: 'tls' }, sends: false, ready_to_send: false })
				]
			})
		);
		expect(view.container.querySelector('#smtp-account')).toBeNull();
		expect(view.getByText(/No account can send yet/).textContent).toContain(
			'Add one with an SMTP server or a Microsoft login under'
		);
		expect(view.getByRole('link', { name: 'Accounts' }).getAttribute('href')).toBe('/settings/accounts');
		stop();
	});

	it("sends as the chosen account's address, with no field to change it", async () => {
		const { view, m, stop } = await page(settingsFixture(THREE, { sending: { enabled: true, account_id: 1 } }));
		expect(view.container.querySelector('#smtp-sender')).toBeNull();
		expect(view.queryByRole('button', { name: 'Use another address' })).toBeNull();
		expect(view.getByText('dmarc@example.com', { selector: 'span' }).parentElement?.textContent).toBe(
			'Sending as dmarc@example.com'
		);

		m.draft.account_id = 2;
		await settle();
		expect(view.getByText('mailer@example.net', { selector: 'span' }).parentElement?.textContent).toBe(
			'Sending as mailer@example.net'
		);
		stop();
	});

	it('says once, at the sender, that the chosen account has no address to send from', async () => {
		const noAddress = settingsFixture(SENDING_ON, { accounts: [accountFixture({ address: '' })] });
		const { view, m, stop } = await page(noAddress);
		expect(view.queryByText(/^Sending as/)).toBeNull();
		expect(m.testBlocked).toBe('Reports has no email address to send from. Enter one under Accounts.');
		expect(view.getAllByText(/has no email address to send from/)).toHaveLength(1);
		expect(view.getByRole('link', { name: 'Accounts' }).getAttribute('href')).toBe('/settings/accounts');
		stop();
	});

	it('without an account chosen, shows no sender', async () => {
		const { view, stop } = await page(THREE);
		expect(view.queryByText('Sender address')).toBeNull();
		stop();
	});

	it('disables the rows below the switch while it is off, and keeps their values', async () => {
		const { input, select, stop } = await page(settingsFixture(THREE, { sending: { account_id: 2 } }));
		const rows = select().closest('fieldset')!;
		expect(rows.disabled).toBe(true);
		expect(select().value).toBe('2');
		// The switch itself is outside the disabled rows.
		expect(input('sending-enabled').closest('fieldset')).toBeNull();
		await fireEvent.click(input('sending-enabled'));
		expect(rows.disabled).toBe(false);
		stop();
	});

	it('switched on without an account: Save is disabled, says why in the warn tone and marks the select', async () => {
		const { view, input, select, m, stop } = await page(THREE);
		await fireEvent.click(input('sending-enabled'));
		const { status, save } = saveBar(view.container);
		expect(save.disabled).toBe(true);
		expect(status?.textContent?.trim()).toBe('Choose an account to send through.');
		expect(status?.querySelector('span')?.className).toContain('text-status-warn-fg');
		expect(select().getAttribute('aria-invalid')).toBe('true');

		m.draft.account_id = 2;
		await settle();
		expect(saveBar(view.container).save.disabled).toBe(false);
		expect(saveBar(view.container).status?.textContent?.trim()).toBe('Unsaved changes');
		expect(select().hasAttribute('aria-invalid')).toBe(false);
		// The account's address is the sender now.
		expect(m.sender).toBe('mailer@example.net');
		stop();
	});
});
