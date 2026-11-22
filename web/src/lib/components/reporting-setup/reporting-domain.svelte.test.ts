// @vitest-environment happy-dom
// A domain's reporting row has its own Re-check in the setup and Settings, and none on the
// domain page, whose one re-check is the DNS scorecard's (it stores the check the row reads).
import { render } from '@testing-library/svelte';
import { describe, expect, it, vi } from 'vitest';
import '#lib/test/component-test.svelte.ts';
import type { ReportingSetupState } from './reporting-setup-state.svelte';
import type { ReportingDomain } from '#lib/types/ReportingDomain.ts';
import ReportingDomainRow from './reporting-domain.svelte';

vi.mock('$app/paths', () => ({ resolve: (path: string) => (path.startsWith('/') ? path : `/${path}`) }));

const ROW: ReportingDomain = {
	id: 7,
	domain: 'example.net',
	last_report_at: null,
	state: 'no_record',
	status: 'fail',
	set_up: false,
	dmarc: null,
	named_addresses: [],
	other_addresses: [],
	missing_authorizations: [],
	refused: [],
	record_to_publish: 'v=DMARC1; p=none; rua=mailto:dmarc@reports.example'
};

function row(recheck?: boolean) {
	const setup = { busy: new Set(), failed: new Map(), lookup: vi.fn() } as unknown as ReportingSetupState;
	const view = render(ReportingDomainRow, {
		row: ROW,
		setup,
		defaultAddress: 'dmarc@reports.example',
		...(recheck === undefined ? {} : { recheck })
	});
	return view.queryByRole('button', { name: /re-check/i });
}

describe('ReportingDomain', () => {
	it('has its own Re-check by default', () => {
		expect(row()).not.toBeNull();
	});

	it('has none where the page re-checks DNS once for everything', () => {
		expect(row(false)).toBeNull();
	});
});
