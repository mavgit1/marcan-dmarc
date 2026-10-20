import { describe, expect, it } from 'vitest';
import { emailDomain, looksLikeEmail } from './report-address';

describe('emailDomain', () => {
	it('returns the lowercased domain without a trailing dot', () => {
		expect(emailDomain('a@Mail.Example.COM.')).toBe('mail.example.com');
	});

	it('uses the last @', () => {
		expect(emailDomain('"a@b"@example.com')).toBe('example.com');
	});

	it('is null without a local part or domain', () => {
		expect(emailDomain('@example.com')).toBeNull();
		expect(emailDomain('a@')).toBeNull();
		expect(emailDomain('no-at-sign')).toBeNull();
	});
});

describe('looksLikeEmail', () => {
	it('accepts a plain address', () => {
		expect(looksLikeEmail('dmarc@reports.example.net')).toBe(true);
	});

	it('rejects what would break a mailto: inside a DMARC record', () => {
		expect(looksLikeEmail('a;b@example.com')).toBe(false);
		expect(looksLikeEmail('a@example.com,b@example.com')).toBe(false);
		expect(looksLikeEmail('a b@example.com')).toBe(false);
		expect(looksLikeEmail('a@localhost')).toBe(false);
	});
});
