// A sender's caption and its evidence are neutral words; a PTR that failed forward confirmation says so.
import { describe, expect, it } from 'vitest';
import type { SenderIdentity } from '$lib/types/SenderIdentity';
import { displayName, identityEvidence, isUnconfirmedOperatorName } from './identity';

const identity = (over: Partial<SenderIdentity>): SenderIdentity =>
	({
		name: null,
		operator_domain: null,
		source: null,
		ptr_hostname: null,
		ptr_confirmed: null,
		spf_domain: null,
		...over
	}) as SenderIdentity;

describe('identityEvidence', () => {
	it('says whether reverse DNS was forward-confirmed', () => {
		const ptr = identity({ source: 'ptr', ptr_hostname: 'mail.example.net', ptr_confirmed: false });
		expect(identityEvidence(ptr)).toBe('Reverse DNS (not forward-confirmed): mail.example.net');
		expect(isUnconfirmedOperatorName(ptr)).toBe(true);
		expect(identityEvidence({ ...ptr, ptr_confirmed: true })).toBe('Forward-confirmed reverse DNS: mail.example.net');
	});

	it('names the SPF domain the reports passed for', () => {
		expect(identityEvidence(identity({ source: 'spf', spf_domain: 'example.net' }))).toBe(
			'SPF passed for example.net in reports'
		);
	});

	it('has nothing to say without evidence', () => {
		expect(identityEvidence(null)).toBeNull();
		expect(displayName(null)).toBe('Unknown');
	});
});
