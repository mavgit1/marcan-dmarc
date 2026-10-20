// The lists edited as text: what saving changes, worded for the "Saving will change" box and the
// save bar's status line.
import { describe, expect, it } from 'vitest';
import type { ClientReportRecipient } from '$lib/types/ClientReportRecipient';
import type { MappingChanges } from '$lib/types/MappingChanges';
import {
	describeMappingChanges,
	describeRecipientChanges,
	describeSenderNameChanges,
	DKIM_SELECTOR_LIST,
	DOMAIN_LIST,
	RECIPIENT_LIST,
	recipientChangeLines
} from './lists';

describe('domain mapping changes', () => {
	const changes = (over: Partial<MappingChanges>) =>
		({
			move: [],
			add: [],
			ignore: [],
			unignore: [],
			create_orgs: [],
			park: [],
			unpark: [],
			...over
		}) as unknown as MappingChanges;

	it('says nothing when nothing changes', () => {
		expect(describeMappingChanges(changes({}))).toBe('');
	});

	it('lists every kind of change and the reports an ignore deletes', () => {
		const c = changes({
			move: [{}, {}],
			add: [{}],
			ignore: [{ reports: 100 }, { reports: 20 }],
			unignore: [{}],
			create_orgs: ['Acme'],
			park: [{}],
			unpark: [{}]
		} as never);
		expect(describeMappingChanges(c)).toBe(
			'2 moved, 1 added, 2 ignored (deletes 120 reports), 1 no longer ignored (past reports are re-imported), 1 new organization, 1 parked, 1 unparked'
		);
	});

	it('leaves out the deletion note when an ignored domain has no reports', () => {
		expect(describeMappingChanges(changes({ ignore: [{ reports: 0 }] } as never))).toBe('1 ignored');
	});

	it('folds the details under their kind', () => {
		const worded = DOMAIN_LIST.changes!({
			changes: changes({
				move: [{ domain: 'a.example', from: 'Old', to: 'New' }],
				ignore: [{ domain: 'b.example', reports: 2 }]
			} as never),
			unchanged_count: 0,
			errors: []
		});
		expect(worded.details.filter((d) => d.items.length > 0)).toEqual([
			{ label: 'Moved', items: ['a.example: Old → New'] },
			{ label: 'Ignored', items: ['b.example: deletes 2 reports'] }
		]);
	});
});

describe('sender name changes', () => {
	it('counts what is added, changed and removed', () => {
		const line = { target: 'crm.example.com', name: 'CRM' };
		expect(describeSenderNameChanges({ add: [line], change: [line, line], remove: ['x'] })).toBe(
			'1 added, 2 changed, 1 removed'
		);
		expect(describeSenderNameChanges({ add: [], change: [], remove: [] })).toBe('');
	});
});

describe('client report recipient changes', () => {
	const r = (organization_id: number, organization: string, email: string): ClientReportRecipient => ({
		organization_id,
		organization,
		email
	});

	it('says who starts and who stops, starts first', () => {
		const changes = {
			start: [r(2, 'Acme', 'it@acme.example')],
			stop: [r(3, 'Beta', 'ops@beta.example'), r(2, 'Acme', 'old@acme.example')],
			unchanged_count: 4
		};
		expect(recipientChangeLines(changes)).toEqual([
			'Starts sending to it@acme.example for Acme',
			'Stops sending to ops@beta.example for Beta',
			'Stops sending to old@acme.example for Acme'
		]);
		expect(describeRecipientChanges(changes)).toBe('1 recipient starts, 2 recipients stop');
	});

	it('says nothing when nothing starts or stops', () => {
		const same = { start: [], stop: [], unchanged_count: 2 };
		expect(recipientChangeLines(same)).toEqual([]);
		expect(describeRecipientChanges(same)).toBe('');
	});
});

describe('what counts as a change', () => {
	it('the recipients ignore surrounding blank space', () => {
		expect(RECIPIENT_LIST.same!('Acme,a@acme.example\n', 'Acme,a@acme.example')).toBe(true);
	});

	it('the DKIM selectors are the same however they are separated', () => {
		expect(DKIM_SELECTOR_LIST.same!('s1, s2', 's1\ns2\n')).toBe(true);
		expect(DKIM_SELECTOR_LIST.same!('s1', 's1 s2')).toBe(false);
	});
});
