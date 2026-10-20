import type { AlertCatalogueItem } from '$lib/types/AlertCatalogueItem';
import type { AlertGroup } from '$lib/types/AlertGroup';
import type { AlertGroupTitle } from '$lib/types/AlertGroupTitle';

/**
 * The server's alert catalogue (`meta.alert_catalogue`, its groups' headings in
 * `meta.alert_groups`): every kind of alert with what triggers it, in groups. Shared by the alert
 * settings (what is emailed) and the alert list (each row's trigger). Every word is the server's.
 */
export type CatalogueGroup = {
	group: AlertGroup;
	title: string;
	entries: AlertCatalogueItem[];
};

/** The catalogue by group: the server's groups, each with its entries in the server's order. */
export function groupCatalogue(
	entries: readonly AlertCatalogueItem[],
	groups: readonly AlertGroupTitle[]
): CatalogueGroup[] {
	return groups
		.map(({ group, title }) => ({
			group,
			title,
			entries: entries.filter((e) => e.group === group)
		}))
		.filter((g) => g.entries.length > 0);
}

/**
 * The folded "Alert kinds" in one line: "All 15 emailed", "12 of 15 emailed", "None of 15
 * emailed". Muted ids the catalogue no longer has do not count.
 */
export function catalogueSummary(entries: readonly AlertCatalogueItem[], muted: readonly string[]): string {
	const total = entries.length;
	const mailed = total - entries.filter((e) => muted.includes(e.id)).length;
	if (mailed === total) return `All ${total} emailed`;
	if (mailed === 0) return `None of ${total} emailed`;
	return `${mailed} of ${total} emailed`;
}

/** The entry with `id`; `undefined` while the catalogue is unknown or for an id it lacks. */
export function catalogueEntry(
	entries: readonly AlertCatalogueItem[] | undefined,
	id: string
): AlertCatalogueItem | undefined {
	return entries?.find((e) => e.id === id);
}

/** What makes an alert row's kind, for its tooltip: the entry's trigger; `undefined` while the catalogue is unknown. */
export function catalogueTrigger(entries: readonly AlertCatalogueItem[] | undefined, id: string): string | undefined {
	return catalogueEntry(entries, id)?.trigger;
}
