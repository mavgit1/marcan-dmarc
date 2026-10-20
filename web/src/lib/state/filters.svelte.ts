import { goto } from '$app/navigation';
import { resolve } from '$app/paths';
import { page } from '$app/state';
import { adoptScope, DEFAULT_SCOPE, scopeParamsFor, writeScope, type Scope } from './scope-url';

export { scopedHref, SINCE_OPTIONS, sinceLabel, sincePhrase } from './scope-url';

/**
 * The scope (organization, domain, time range) while the app runs; the URL keeps it for reloads
 * and bookmarks (`scope-url`). Pages read `filters`, never the URL, and no click handler sets
 * it: a link that narrows names the scope (`scopedHref`), so the narrowing shows in the URL and
 * survives a new tab. Picking a domain in the header opens its page, except on `/alerts`, which
 * narrows in place. Code that changes `filters` without navigating calls `syncScopeUrl()`.
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
 * scope by itself: an organization page shows its organization because of its route, and its
 * links name the organization (`scopedHref`). The root layout calls it.
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
 * none). Called after every navigation and after the scope filter changes without one.
 */
export function syncScopeUrl() {
	const params = scopeParamsFor(page.url.pathname);
	if (params.length === 0) return;
	const search = writeScope(page.url.searchParams, filters, params);
	if (search.toString() === page.url.searchParams.toString()) return;
	const query = search.toString();
	const href = `${page.url.pathname}${query ? `?${query}` : ''}${page.url.hash}`;
	void goto(resolve(href as '/'), { replaceState: true, keepFocus: true, noScroll: true });
}
