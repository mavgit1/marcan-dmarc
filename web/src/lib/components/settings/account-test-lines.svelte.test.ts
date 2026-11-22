// @vitest-environment happy-dom
// The test's lines name the mailbox step by what it reads the mailbox over: IMAP for a password
// login, the mailbox for a Microsoft login (read over Microsoft Graph), beside its token line.
import { render } from '@testing-library/svelte';
import { describe, expect, it } from 'vitest';
import type { AccountTestResponse } from '#lib/types/AccountTestResponse.ts';
import AccountTestLines from './account-test-lines.svelte';

const mailbox = {
	ok: true,
	message: 'Logged in as dmarc@example.com.',
	detail: null,
	docs: null,
	folders: [
		{ name: 'INBOX', delimiter: '/' },
		{ name: 'DMARC', delimiter: '/' }
	],
	selected: { name: 'INBOX', exists: true, messages: 3 }
};

const lines = (container: HTMLElement) => [...container.querySelectorAll('li')].map((li) => li.textContent?.trim());

describe('AccountTestLines', () => {
	it('says IMAP for a password login, with its SMTP line', () => {
		const result: AccountTestResponse = {
			token: null,
			mailbox,
			smtp: { ok: true, message: 'Logged in as dmarc@example.com.', detail: null, docs: null }
		};
		const { container } = render(AccountTestLines, { result, login: 'password' });
		expect(lines(container)).toEqual([
			'IMAP: Logged in as dmarc@example.com. 2 folders listed.',
			'SMTP: Logged in as dmarc@example.com.'
		]);
	});

	it('says Mailbox for a Microsoft login, after its token, and no IMAP or SMTP', () => {
		const result: AccountTestResponse = {
			token: {
				ok: true,
				message: 'Microsoft issued a token for the app registration.',
				detail: null,
				docs: null
			},
			mailbox,
			smtp: null
		};
		const { container } = render(AccountTestLines, { result, login: 'microsoft' });
		const text = lines(container);
		expect(text).toEqual([
			'Token: Microsoft issued a token for the app registration.',
			'Mailbox: Logged in as dmarc@example.com. 2 folders listed.'
		]);
		expect(container.textContent).not.toMatch(/IMAP|SMTP/);
	});

	it('ends a failed line with a link to the docs section the server names', () => {
		const result: AccountTestResponse = {
			token: { ok: true, message: 'Microsoft issued a token for the app registration.', detail: null, docs: null },
			mailbox: {
				ok: false,
				message: 'The app registration may not read dmarc@example.com. Test again later.',
				detail: 'ErrorAccessDenied: Access is denied.',
				docs: 'accounts#exchange-online',
				folders: [],
				selected: null
			},
			smtp: null
		};
		const { container } = render(AccountTestLines, { result, login: 'microsoft' });
		const failed = container.querySelectorAll('li')[1];
		expect(failed.textContent).toContain('Test again later (docs).');
		const link = failed.querySelector('a');
		expect(link?.getAttribute('href')).toBe('https://marcan.kaitiaki.tech/accounts#exchange-online');
		expect(link?.textContent).toBe('docs');
		expect(container.querySelectorAll('li')[0].querySelector('a')).toBeNull();
	});
});
