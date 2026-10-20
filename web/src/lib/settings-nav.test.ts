// Links into Settings: which page a path shows and where a link to a page or section points.
import { describe, expect, it, vi } from 'vitest';
import {
	pageFromPath,
	settingsAnchor,
	settingsHref,
	settingsTitle,
	SETTINGS_GROUPS,
	SETTINGS_PAGES,
	SETTINGS_SECTIONS,
	type SettingsSectionId
} from './settings-nav';

vi.mock('$app/paths', () => ({ resolve: (path: string) => path }));

describe('pageFromPath', () => {
	it('names the page of each Settings path', () => {
		for (const { id } of SETTINGS_PAGES) expect(pageFromPath(`/settings/${id}`)).toBe(id);
		expect(pageFromPath('/settings/alerts/')).toBe('alerts');
	});

	it('is undefined on the list and elsewhere', () => {
		expect(pageFromPath('/settings')).toBeUndefined();
		expect(pageFromPath('/settings/nope')).toBeUndefined();
		expect(pageFromPath('/alerts')).toBeUndefined();
	});
});

describe('links', () => {
	it('opens a page at its own URL, without an anchor', () => {
		expect(settingsHref('mailbox')).toBe('/settings/mailbox');
		expect(settingsHref('dns-checks')).toBe('/settings/dns-checks');
		expect(settingsAnchor('alerts')).toBeUndefined();
	});

	it('opens a section on its page, at its anchor', () => {
		expect(settingsHref('retention')).toBe('/settings/data#retention');
		expect(settingsHref('dns-resolvers')).toBe('/settings/dns-checks#dns-resolvers');
		expect(settingsHref('client-report-recipients')).toBe('/settings/client-reports#client-report-recipients');
		expect(settingsHref('alert-kinds')).toBe('/settings/alerts#alert-kinds');
	});

	it('words a link by its title', () => {
		expect(settingsTitle('client-reports')).toBe('Client reports');
		expect(settingsTitle('client-report-recipients')).toBe('Recipients');
	});

	it('keeps page and section ids apart, and every page in a group that exists', () => {
		const pages = new Set<string>(SETTINGS_PAGES.map((p) => p.id));
		expect(pages.size).toBe(SETTINGS_PAGES.length);
		for (const id of Object.keys(SETTINGS_SECTIONS)) expect(pages.has(id)).toBe(false);
		for (const id of Object.keys(SETTINGS_SECTIONS) as SettingsSectionId[])
			expect(pages.has(SETTINGS_SECTIONS[id].page)).toBe(true);
		const groups = new Set<string>(SETTINGS_GROUPS.map((g) => g.id));
		for (const page of SETTINGS_PAGES) expect(groups.has(page.group)).toBe(true);
	});
});
