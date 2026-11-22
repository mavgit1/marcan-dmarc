import { goto } from '$app/navigation';
import { resolve } from '$app/paths';
import { page } from '$app/state';
import { adoptScope, DEFAULT_SCOPE, scopeParamsFor, writeScope, type Scope } from './scope-url';

export { scopedHref, SINCE_OPTIONS, sinceLabel, sincePhrase } from './scope-url';

/**
 * The scope (organization, domain, time range) while the app runs; the URL keeps it for reloads
 * and bookmarks (`scope-url`). It narrows the lists and the dashboard only: a page about one
 * organization, domain or sender reads just the time range. Lists read `filters`, never the URL.
 * Only the header filter sets it, or a link to a list that names a scope on purpose
 * (`scopedHref`), which is the same as picking it there. Picking a domain in the header opens
 * its page, except on `/alerts`, which narrows in place. Code that changes `filters` without
 * navigating calls `syncScopeUrl()`.
 */
class Filters {
	organizationId = $state<number | null>(DEFAULT_SCOPE.organizationId);
	domainId = $state<number | null>(DEFAULT_SCOPE.domainId);
	since = $state(DEFAULT_SCOPE.since);
}

export const filters = new Filters();

/**
 * Takes the scope from a URL the app navigated to (see `adoptScope`): `reset` on the first load
 * and on back / forward, `carry` on an in-app link or `goto`. Opening a page never changes the
 * scope: an organization, domain or sender page shows its entity because of its route and reads
 * no `org` or `domain` from its URL. The root layout calls it.
 */
export function adoptScopeUrl(url: URL, mode: 'reset' | 'carry') {
	const current: Scope = { organizationId: filters.organizationId, domainId: filters.domainId, since: filters.since };
	const next = adoptScope(current, url.searchParams, scopeParamsFor(url.pathname), mode);
	if (next.organizationId !== filters.organizationId) filters.organizationId = next.organizationId;
	if (next.domainId !== filters.domainId) filters.domainId = next.domainId;
	if (next.since !== filters.since) filters.since = next.since;
}

/**
 * Writes the scope into the current URL, replacing the history entry (a filter change adds
 * none). Called after every navigation and after the scope filter changes without one; settles
 * once the URL is written.
 */
export async function syncScopeUrl(): Promise<void> {
	const params = scopeParamsFor(page.url.pathname);
	if (params.length === 0) return;
	const search = writeScope(new URLSearchParams(page.url.search), filters, params);
	if (search.toString() === page.url.searchParams.toString()) return;
	const query = search.toString();
	const href = `${page.url.pathname}${query ? `?${query}` : ''}${page.url.hash}`;
	await goto(resolve(href as '/'), { replace: true, reset: false });
}
