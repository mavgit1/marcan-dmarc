// The account draft's rules: the stored password applies only to its own servers and username,
// the request carries a password only when typed, and a Microsoft login carries no servers.
import { describe, expect, it } from 'vitest';
import { accountFixture, settingsFixture } from '$lib/test/settings-fixture';
import { fromAccount, loginKey, normalize, sameTargets, toRequest, withPorts } from './account-draft.svelte';

const saved = fromAccount(accountFixture(), settingsFixture());

describe('account draft', () => {
	it('keeps the stored password for the same servers and username, and when a server goes', () => {
		expect(sameTargets({ ...saved, imapHost: 'IMAP.example.com' }, saved)).toBe(true);
		expect(sameTargets({ ...saved, smtpHost: '' }, saved)).toBe(true);
		expect(sameTargets({ ...saved, imapHost: 'imap.example.net' }, saved)).toBe(false);
		expect(sameTargets({ ...saved, smtpPort: 587 }, saved)).toBe(false);
		expect(sameTargets({ ...saved, username: 'other' }, saved)).toBe(false);
		// A server added where there was none is a new target.
		const noSmtp = { ...saved, smtpHost: '' };
		expect(sameTargets(saved, noSmtp)).toBe(false);
	});

	it('compares an emptied port as the port the server fills in', () => {
		const s = settingsFixture();
		const emptied = withPorts({ ...saved, imapPort: null, smtpPort: null }, s);
		expect(emptied).toMatchObject({ imapPort: 993, smtpPort: 465 });
		expect(sameTargets(emptied, withPorts(saved, s))).toBe(true);
		// STARTTLS: its usual ports, also where the preset's security is TLS (Google's).
		expect(withPorts({ ...saved, imapPort: null, imapSecurity: 'starttls' }, s).imapPort).toBe(143);
		const google = { ...saved, provider: 'google' as const, smtpPort: null, smtpSecurity: 'starttls' as const };
		expect(withPorts(google, s).smtpPort).toBe(587);
		// No SMTP host: no port.
		expect(withPorts({ ...saved, smtpHost: '', smtpPort: null }, s).smtpPort).toBeNull();
	});

	it('sends a password only when typed, and empty ports as none', () => {
		const request = toRequest(normalize({ ...saved, imapPort: '' as unknown as number, smtpPort: null }));
		expect(request.login).toEqual({ kind: 'password', username: 'dmarc@example.com', password: undefined });
		expect(request.imap?.port).toBe(0);
		expect(request.smtp?.port).toBeNull();
		expect(request.receiving.forwarded_from).toEqual([]);
		const typed = toRequest(
			normalize({ ...saved, password: ' secret ', forwardedFrom: [' a@example.net ', '', 'b@example.org'] })
		);
		expect(typed.login).toEqual({ kind: 'password', username: 'dmarc@example.com', password: ' secret ' });
		expect(typed.receiving.forwarded_from).toEqual(['a@example.net', 'b@example.org']);
	});

	it('words a Microsoft login with its certificate and without servers', () => {
		const ms = {
			...saved,
			provider: 'microsoft365' as const,
			loginKind: 'microsoft' as const,
			tenantId: 't',
			clientId: 'c'
		};
		const request = toRequest(normalize(ms));
		expect(request.login).toEqual({ kind: 'microsoft', tenant_id: 't', client_id: 'c', draft_id: undefined });
		expect([request.imap, request.smtp]).toEqual([null, null]);
		const certificate = { draft_id: 'd1', thumbprint: 'AB', expires_at: 1, subject: 'dmarc@example.com' };
		expect(toRequest(normalize({ ...ms, certificate })).login).toMatchObject({ draft_id: 'd1' });
		// Its server fields are no change and no part of what a test tries; its address is.
		expect(normalize({ ...ms, imapHost: 'imap.example.net', smtpPort: 25 })).toEqual(normalize(ms));
		expect(loginKey(normalize({ ...ms, imapHost: 'imap.example.net' }))).toBe(loginKey(normalize(ms)));
		expect(loginKey(normalize({ ...ms, address: 'other@example.com' }))).not.toBe(loginKey(normalize(ms)));
	});
});
