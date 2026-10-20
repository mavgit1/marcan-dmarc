import { resolve } from '$app/paths';

/**
 * Where things are in Settings: its pages (`/settings/<page>`, grouped in the nav column), the
 * sections inside a page that links point at, and the links to both. Every link into Settings
 * goes through `settingsHref` or the `settings/SettingsLink` component, so a moved section is
 * one edit here.
 */
export const SETTINGS_GROUPS = [
	{ id: 'reports-in', label: 'Reports in' },
	{ id: 'email-out', label: 'Email out' },
	{ id: 'domains-dns', label: 'Domains and DNS' },
	{ id: 'installation', label: 'This installation' }
] as const;

export type SettingsGroupId = (typeof SETTINGS_GROUPS)[number]['id'];

/** Every Settings page in nav order: its title (what links to it say) and its group. */
export const SETTINGS_PAGES = [
	{ id: 'mailbox', title: 'Mailbox', group: 'reports-in' },
	{ id: 'sending', title: 'Sending', group: 'email-out' },
	{ id: 'alerts', title: 'Alerts', group: 'email-out' },
	{ id: 'client-reports', title: 'Client reports', group: 'email-out' },
	{ id: 'domains', title: 'Domains', group: 'domains-dns' },
	{ id: 'sender-names', title: 'Sender names', group: 'domains-dns' },
	{ id: 'dns-checks', title: 'DNS checks', group: 'domains-dns' },
	{ id: 'branding', title: 'Branding', group: 'installation' },
	{ id: 'license', title: 'License', group: 'installation' },
	{ id: 'data', title: 'Data', group: 'installation' }
] as const satisfies readonly { id: string; title: string; group: SettingsGroupId }[];

export type SettingsPageId = (typeof SETTINGS_PAGES)[number]['id'];

/** A section inside a page that a link can open: its title and its page. Its id is its anchor. */
export const SETTINGS_SECTIONS = {
	'alert-kinds': { title: 'Alert kinds', page: 'alerts' },
	'client-report-recipients': { title: 'Recipients', page: 'client-reports' },
	'dns-resolvers': { title: 'Resolvers', page: 'dns-checks' },
	'dkim-selectors': { title: 'Extra DKIM selectors', page: 'dns-checks' },
	'operator-key': { title: 'Operator key', page: 'license' },
	'stored-data': { title: 'Stored data', page: 'data' },
	retention: { title: 'Retention', page: 'data' }
} as const satisfies Record<string, { title: string; page: SettingsPageId }>;

export type SettingsSectionId = keyof typeof SETTINGS_SECTIONS;

/** What a link into Settings opens: a page, or a section on one. */
export type SettingsTarget = SettingsPageId | SettingsSectionId;

/** The page `id`, or `undefined` for anything else. */
export function settingsPage(id: string | null | undefined): (typeof SETTINGS_PAGES)[number] | undefined {
	return SETTINGS_PAGES.find((p) => p.id === id);
}

/** The page a Settings path shows (`/settings/alerts` → alerts); `undefined` on the list itself. */
export function pageFromPath(pathname: string): SettingsPageId | undefined {
	const match = /^\/settings\/([^/]+)\/?$/.exec(pathname);
	return settingsPage(match?.[1])?.id;
}

function isSection(target: SettingsTarget): target is SettingsSectionId {
	return target in SETTINGS_SECTIONS;
}

/** The anchor a link opens: a section's id, nothing for a whole page. */
export function settingsAnchor(target: SettingsTarget): string | undefined {
	return isSection(target) ? target : undefined;
}

/** What a link to `target` says by default: the page's or the section's title. */
export function settingsTitle(target: SettingsTarget): string {
	return isSection(target) ? SETTINGS_SECTIONS[target].title : settingsPage(target)!.title;
}

/** The link that opens `target`: its page, scrolled to the section when it is one. */
export function settingsHref(target: SettingsTarget): string {
	const page = isSection(target) ? SETTINGS_SECTIONS[target].page : target;
	const anchor = settingsAnchor(target);
	return `${resolve(`/settings/${page}`)}${anchor ? `#${anchor}` : ''}`;
}
