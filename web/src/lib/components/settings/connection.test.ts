// The mailbox and the sending account share one rule for the security choice, its port and
// whether the saved password applies.
import { describe, expect, it } from 'vitest';
import { defaultPort, portAfterSecurityChange, sameAccount, SECURITY_LABEL } from './connection';

const IMAP = { port: 993, starttls_port: 143 };
const SMTP = { port: 465, starttls_port: 587 };

describe('security and port', () => {
	it('words each choice without a number: the port field shows it', () => {
		expect(SECURITY_LABEL).toEqual({ tls: 'TLS', starttls: 'STARTTLS' });
	});

	it("knows each choice's usual port", () => {
		expect(defaultPort('tls', IMAP)).toBe(993);
		expect(defaultPort('starttls', SMTP)).toBe(587);
	});

	it('moves the port while it is empty or the usual one of the old choice', () => {
		expect(portAfterSecurityChange(993, 'tls', 'starttls', IMAP)).toBe(143);
		expect(portAfterSecurityChange(143, 'starttls', 'tls', IMAP)).toBe(993);
		expect(portAfterSecurityChange(465, 'tls', 'starttls', SMTP)).toBe(587);
		// Empty: nothing, or 0.
		expect(portAfterSecurityChange(null, 'tls', 'starttls', SMTP)).toBe(587);
		expect(portAfterSecurityChange(0, 'tls', 'starttls', IMAP)).toBe(143);
	});

	it('keeps a port the user typed', () => {
		expect(portAfterSecurityChange(1143, 'tls', 'starttls', IMAP)).toBe(1143);
		expect(portAfterSecurityChange(2525, 'tls', 'starttls', SMTP)).toBe(2525);
		// 143 typed while on TLS is the user's port, not TLS's usual one.
		expect(portAfterSecurityChange(143, 'tls', 'starttls', IMAP)).toBe(143);
		expect(portAfterSecurityChange(993, 'tls', 'tls', IMAP)).toBe(993);
	});
});

describe('sameAccount', () => {
	const a = { host: 'mail.example.com', port: 993, username: 'dmarc@example.com' };

	it('ignores the host case, not the port or the username', () => {
		expect(sameAccount(a, { ...a, host: 'MAIL.EXAMPLE.COM' })).toBe(true);
		expect(sameAccount(a, { ...a, port: 143 })).toBe(false);
		expect(sameAccount(a, { ...a, username: 'DMARC@example.com' })).toBe(false);
	});
});
