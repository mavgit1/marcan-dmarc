// @vitest-environment happy-dom
// The Guide's first blocks as they render: the addresses reports reach Marcan at (with the ones
// forwarded to an account) and how many domains report there, or, with no receiving account,
// that none does and where to add one; then every receiving domain's permission.
import { render } from '@testing-library/svelte';
import { describe, expect, it, vi } from 'vitest';
import '$lib/test/component-test.svelte';
import type { ReportingSetupState } from '$lib/components/reporting-setup/reporting-setup-state.svelte';
import type { AddressAuthorization } from '$lib/types/AddressAuthorization';
import type { ReportingDomain } from '$lib/types/ReportingDomain';
import type { ReportingSetupResponse } from '$lib/types/ReportingSetupResponse';
import GuideReporting from './guide-reporting.svelte';

vi.mock('$app/paths', () => ({ resolve: (path: string) => path }));

const auth = (
	domain: string,
	state: AddressAuthorization['state'],
	status: AddressAuthorization['status'] = 'neutral'
): AddressAuthorization => ({ domain, org_domain: domain, addresses: [], state, status });

const row = (id: number, domain: string, state: ReportingDomain['state'], missing: string[] = []) =>
	({
		id,
		domain,
		state,
		status: state === 'reporting' ? 'pass' : 'fail',
		set_up: state === 'reporting',
		missing_authorizations: missing
	}) as ReportingDomain;

const TWO: ReportingSetupResponse = {
	addresses: [
		{ account_id: 1, name: 'Reports', address: 'dmarc@reports.example.net', forwarded_from: [] },
		{ account_id: 2, name: 'Second', address: 'reports@other.example', forwarded_from: ['dmarc@example.org'] }
	],
	default_address: 'dmarc@reports.example.net',
	authorizations: [
		auth('reports.example.net', 'no_wildcard'),
		auth('other.example', 'wildcard', 'pass'),
		auth('example.org', 'not_needed')
	],
	domains: [
		row(1, 'example.com', 'reporting'),
		row(2, 'example.net', 'no_record', ['reports.example.net']),
		row(3, 'example.org', 'reporting')
	]
};

function guide(data: ReportingSetupResponse | null) {
	const reporting = {
		data,
		source: { error: null, loading: false },
		recheck: vi.fn()
	} as unknown as ReportingSetupState;
	const view = render(GuideReporting, { reporting });
	const block = (id: string) =>
		(view.container.querySelector(`section[aria-labelledby="guide-${id}"]`)?.textContent ?? '').replace(/\s+/g, ' ');
	return { view, block };
}

describe('GuideReporting', () => {
	it('names every address reports reach Marcan at, the forwarded one too, and the reporting count', () => {
		const { block } = guide(TWO);
		expect(block('reach')).toContain(
			'Reports reach Marcan at dmarc@reports.example.net and reports@other.example (forwarded from dmarc@example.org).'
		);
		expect(block('reach')).toContain('new records name dmarc@reports.example.net');
		expect(block('reach')).toContain('2 of 3 domains report here.');
		expect(block('reach')).toContain('Not yet: example.net (no record).');
	});

	it('names a few forwarded addresses of an account and counts many', () => {
		const few = ['dmarc@client1.example', 'dmarc@client2.example', 'dmarc@client3.example'];
		const second = (forwarded_from: string[]) => ({ ...TWO.addresses[1], forwarded_from });
		let { block } = guide({ ...TWO, addresses: [TWO.addresses[0], second(few)] });
		expect(block('reach')).toContain(
			'reports@other.example (forwarded from dmarc@client1.example, dmarc@client2.example and dmarc@client3.example).'
		);
		({ block } = guide({ ...TWO, addresses: [TWO.addresses[0], second([...few, 'dmarc@client4.example'])] }));
		expect(block('reach')).toContain('reports@other.example (and 4 addresses forwarded to it).');
	});

	it('with one address says it is the one to name', () => {
		const { block } = guide({ ...TWO, addresses: [TWO.addresses[0]] });
		expect(block('reach')).toContain(
			'Reports reach Marcan at dmarc@reports.example.net. Every domain you monitor must name it'
		);
		expect(block('reach')).not.toContain('new records name');
	});

	it("lists every receiving domain's permission", () => {
		const { view, block } = guide(TWO);
		const lines = [...view.container.querySelectorAll('section[aria-labelledby="guide-permission"] li')].map((li) =>
			li.textContent?.replace(/\s+/g, ' ').trim()
		);
		expect(lines).toEqual([
			'reports.example.net: No wildcard, 1 domain without permission. Publish it in the setup',
			'other.example: Authorized.',
			'example.org: Not needed.'
		]);
		const fix = view.getByRole('link', { name: 'Publish it in the setup' });
		expect(fix.getAttribute('href')).toBe('/setup?step=addresses');
		expect(block('permission')).not.toContain('Check failed');
	});

	it('without a receiving account says so and links to Accounts', () => {
		const { view, block } = guide({ addresses: [], default_address: '', authorizations: [], domains: [] });
		expect(block('reach')).toContain('No account receives reports yet');
		const link = view.getByRole('link', { name: 'add one under Settings → Accounts' });
		expect(link.getAttribute('href')).toContain('/settings/accounts');
		expect(view.container.querySelector('section[aria-labelledby="guide-permission"] li')).toBeNull();
	});
});
