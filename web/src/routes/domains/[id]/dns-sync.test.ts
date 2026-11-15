import { describe, expect, it } from 'vitest';
import { dnsCheckedAt } from './dns-sync';

describe('dnsCheckedAt', () => {
	it('is the stored check’s whole unix seconds', () => {
		expect(dnsCheckedAt('2026-10-01T16:00:37Z')).toBe(1_790_870_437);
		expect(dnsCheckedAt('2026-10-01T16:00:37.912345Z')).toBe(1_790_870_437);
		expect(dnsCheckedAt('2026-10-01T18:00:37+02:00')).toBe(1_790_870_437);
	});

	it('is null when it does not parse', () => {
		expect(dnsCheckedAt('soon')).toBeNull();
	});
});
