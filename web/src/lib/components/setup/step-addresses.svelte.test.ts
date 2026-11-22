// @vitest-environment happy-dom
// The setup's Report addresses step as it renders: each receiving account's address with the
// ones forwarded to it, the address new records name, and the authorization records to publish
// for each address domain that needs one, with the real names; nothing to type.
import { render } from '@testing-library/svelte';
import { describe, expect, it, vi } from 'vitest';
import '#lib/test/component-test.svelte.ts';
import type { ReportingSetupState } from '#lib/components/reporting-setup/reporting-setup-state.svelte.ts';
import type { AddressAuthorization } from '#lib/types/AddressAuthorization.ts';
import type { ReportingSetupResponse } from '#lib/types/ReportingSetupResponse.ts';
import StepAddresses from './step-addresses.svelte';

vi.mock('$app/paths', () => ({ resolve: (path: string) => (path.startsWith('/') ? path : `/${path}`) }));

const auth = (
	domain: string,
	addresses: string[],
	state: AddressAuthorization['state'],
	status: AddressAuthorization['status'] = 'neutral'
): AddressAuthorization => ({ domain, org_domain: domain, addresses, state, status });

/** Two accounts, the second also receiving the reports forwarded from `dmarc@example.org` and `dmarc@client.example`. */
const TWO: ReportingSetupResponse = {
	addresses: [
		{ account_id: 1, name: 'Reports', address: 'dmarc@reports.example.net', forwarded_from: [] },
		{
			account_id: 2,
			name: 'Second',
			address: 'reports@other.example',
			forwarded_from: ['dmarc@example.org', 'dmarc@client.example']
		}
	],
	default_address: 'dmarc@reports.example.net',
	authorizations: [
		auth('reports.example.net', ['dmarc@reports.example.net'], 'no_wildcard'),
		auth('other.example', ['reports@other.example'], 'wildcard', 'pass'),
		auth('example.org', ['dmarc@example.org'], 'not_needed')
	],
	domains: []
};

/** A reporting setup that answered `data` (or is still out with `null`). */
function setup(data: ReportingSetupResponse | null): ReportingSetupState {
	return { data, source: { error: null, loading: false }, recheck: vi.fn() } as unknown as ReportingSetupState;
}

function step(data: ReportingSetupResponse | null) {
	return render(StepAddresses, { reporting: setup(data), onBack: vi.fn(), onNext: vi.fn() });
}

describe('StepAddresses', () => {
	it('lists every receiving account with the addresses forwarded to it', () => {
		const view = step(TWO);
		const rows = [...view.getByRole('list', { name: 'Report addresses' }).querySelectorAll('li')].map((li) =>
			li.textContent?.replace(/\s+/g, ' ').trim()
		);
		expect(rows).toEqual([
			'Reports Reports sent to dmarc@reports.example.net land here',
			'Second Reports sent to reports@other.example land here Also reports forwarded from dmarc@example.org and dmarc@client.example'
		]);
		expect(view.container.textContent).toContain('New DMARC records name dmarc@reports.example.net, the first account');
		// Nothing to type.
		expect(view.container.querySelector('input')).toBeNull();
	});

	it('shows the records each address domain must publish, with the real names, and only those', () => {
		const view = step(TWO);
		const parts = [...view.container.querySelectorAll('section')].map((s) => s.getAttribute('aria-label'));
		expect(parts).toEqual(['Authorization of reports.example.net', 'Authorization of other.example']);
		const values = [...view.container.querySelectorAll('section')].map((s) => s.textContent ?? '');
		expect(values[0]).toContain('*._report._dmarc.reports.example.net');
		expect(values[0]).toContain('v=DMARC1');
		expect(values[0]).toContain('No wildcard');
		expect(values[1]).toContain('*._report._dmarc.other.example is published');
		expect(view.container.textContent).not.toContain('<other-domain>');
	});

	it('says when no address domain needs a record', () => {
		const one: ReportingSetupResponse = {
			...TWO,
			addresses: [TWO.addresses[0]],
			authorizations: [auth('reports.example.net', ['dmarc@reports.example.net'], 'not_needed')]
		};
		const view = step(one);
		expect(view.container.querySelectorAll('section')).toHaveLength(0);
		expect(view.container.textContent).toContain('Not needed: dmarc@reports.example.net is on reports.example.net');
		expect(view.container.textContent).not.toContain('the first account above');
	});

	it('without a receiving account says what is missing instead of an example', () => {
		const view = step({ addresses: [], default_address: '', authorizations: [], domains: [] });
		expect(view.container.textContent).toContain('No account receives reports yet: add one in the Accounts step.');
		expect(view.container.textContent).not.toContain('@');
	});
});
