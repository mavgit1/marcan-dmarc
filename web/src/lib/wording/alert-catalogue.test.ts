// The alert catalogue's groups: in the server's order, every entry once, under the server's headings.
import { describe, expect, it } from 'vitest';
import type { AlertCatalogueItem } from '$lib/types/AlertCatalogueItem';
import type { AlertGroupTitle } from '$lib/types/AlertGroupTitle';
import { catalogueEntry, catalogueSummary, catalogueTrigger, groupCatalogue } from './alert-catalogue';

const entry = (id: string, group: AlertCatalogueItem['group']): AlertCatalogueItem => ({
	id,
	group,
	title: id,
	trigger: `${id} happened.`
});

const CATALOGUE = [
	entry('issue_opened.reports_stopped', 'issues'),
	entry('issue_resolved.reports_stopped', 'issues'),
	entry('health_changed', 'health'),
	entry('dns_record_changed.spf', 'dns_records'),
	entry('dns_record_changed.dkim', 'dns_records'),
	entry('policy_changed', 'policy')
];

const GROUPS: AlertGroupTitle[] = [
	{ group: 'issues', title: 'Issues' },
	{ group: 'health', title: 'Health' },
	{ group: 'dns_records', title: 'DNS records' },
	{ group: 'policy', title: 'Policy' },
	{ group: 'parked', title: 'Parked' }
];

describe('groupCatalogue', () => {
	it('keeps the server order of groups and entries, with their headings', () => {
		const groups = groupCatalogue(CATALOGUE, GROUPS);
		expect(groups.map((g) => g.group)).toEqual(['issues', 'health', 'dns_records', 'policy']);
		expect(groups[2].title).toBe('DNS records');
		expect(groups[2].entries.map((e) => e.id)).toEqual(['dns_record_changed.spf', 'dns_record_changed.dkim']);
		expect(groups.flatMap((g) => g.entries)).toHaveLength(CATALOGUE.length);
	});

	it('finds an entry by id', () => {
		expect(catalogueEntry(CATALOGUE, 'health_changed')?.group).toBe('health');
		expect(catalogueEntry(CATALOGUE, 'gone')).toBeUndefined();
		expect(catalogueEntry(undefined, 'health_changed')).toBeUndefined();
	});

	it('gives the trigger of an entry', () => {
		expect(catalogueTrigger(CATALOGUE, 'health_changed')).toBe('health_changed happened.');
		expect(catalogueTrigger(undefined, 'health_changed')).toBeUndefined();
	});
});

describe('catalogueSummary', () => {
	it('says how many kinds are emailed', () => {
		expect(catalogueSummary(CATALOGUE, [])).toBe('All 6 emailed');
		expect(catalogueSummary(CATALOGUE, ['health_changed', 'policy_changed', 'dns_record_changed.spf'])).toBe(
			'3 of 6 emailed'
		);
		expect(
			catalogueSummary(
				CATALOGUE,
				CATALOGUE.map((e) => e.id)
			)
		).toBe('None of 6 emailed');
	});

	it('ignores muted kinds the catalogue no longer has', () => {
		expect(catalogueSummary(CATALOGUE, ['gone'])).toBe('All 6 emailed');
		expect(catalogueSummary(CATALOGUE, ['gone', 'health_changed'])).toBe('5 of 6 emailed');
	});
});
