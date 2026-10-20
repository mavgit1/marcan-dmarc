// The scope filter in the URL: which pages carry which parameters, parsing, writing, defaults.
import { describe, expect, it } from 'vitest';
import {
	adoptScope,
	DEFAULT_SCOPE,
	readScope,
	scopedHref,
	scopeParamsFor,
	sincePhrase,
	writeScope,
	type Scope
} from './scope-url';

const ALL = ['org', 'domain', 'since'] as const;
const q = (s: string) => new URLSearchParams(s);
const scope = (patch: Partial<Scope> = {}): Scope => ({ ...DEFAULT_SCOPE, ...patch });

describe('scopeParamsFor', () => {
	it('carries the whole scope on the scoped lists and the dashboard', () => {
		for (const path of ['/', '/alerts', '/reports', '/senders', '/senders/']) {
			expect(scopeParamsFor(path)).toEqual(ALL);
		}
	});

	it('leaves out what a page pins or does not read', () => {
		expect(scopeParamsFor('/domains')).toEqual(['org', 'since']);
		expect(scopeParamsFor('/senders/203.0.113.9')).toEqual(['org', 'since']);
		expect(scopeParamsFor('/domains/12')).toEqual(['org', 'since']);
		expect(scopeParamsFor('/organizations')).toEqual(['since']);
		expect(scopeParamsFor('/organizations/3')).toEqual(['since']);
	});

	it('carries nothing where the scope does not apply', () => {
		for (const path of ['/settings', '/tools/dns', '/guide', '/reports/5', '/nowhere']) {
			expect(scopeParamsFor(path)).toEqual([]);
		}
	});
});

describe('readScope', () => {
	it('parses every parameter', () => {
		expect(readScope(q('org=3&domain=12&since=7d'), ALL)).toEqual({ organizationId: 3, domainId: 12, since: '7d' });
	});

	it('ignores invalid values', () => {
		expect(readScope(q('org=abc&domain=0&since=1y'), ALL)).toEqual({});
		expect(readScope(q('org=-1&domain=1.5&since='), ALL)).toEqual({});
		expect(readScope(q('org=007&domain=99999999999999999999'), ALL)).toEqual({});
	});

	it('reads only the parameters the page carries', () => {
		expect(readScope(q('org=3&domain=12&since=90d'), ['since'])).toEqual({ since: '90d' });
	});
});

describe('adoptScope', () => {
	const current = scope({ organizationId: 3, domainId: 12, since: '7d' });

	it('takes the URL as the source on reset; an absent parameter is its default', () => {
		expect(adoptScope(current, q(''), ALL, 'reset')).toEqual(DEFAULT_SCOPE);
		expect(adoptScope(current, q('since=90d'), ALL, 'reset')).toEqual(scope({ since: '90d' }));
		expect(adoptScope(current, q('org=x'), ALL, 'reset')).toEqual(DEFAULT_SCOPE);
	});

	it('keeps the current scope for an in-app link that does not name it', () => {
		expect(adoptScope(current, q(''), ALL, 'carry')).toEqual(current);
		expect(adoptScope(current, q('since=30d'), ALL, 'carry')).toEqual({ ...current, since: '30d' });
		expect(adoptScope(current, q('since=never'), ALL, 'carry')).toEqual(current);
	});

	it('keeps what the page does not carry, either way', () => {
		expect(adoptScope(current, q('org=5'), ['since'], 'reset')).toEqual({ ...current, since: '30d' });
	});

	it('never narrows the scope for opening an organization page', () => {
		const all = scope();
		for (const mode of ['reset', 'carry'] as const) {
			expect(adoptScope(all, q('org=4'), scopeParamsFor('/organizations/4'), mode)).toEqual(all);
		}
	});

	it('narrows to the organization a link from its page names', () => {
		const url = new URL(scopedHref('/domains/9', { organizationId: 4 }), 'http://marcan.test');
		expect(adoptScope(scope(), url.searchParams, scopeParamsFor(url.pathname), 'carry')).toEqual(
			scope({ organizationId: 4 })
		);
	});
});

describe('scopedHref', () => {
	it('names the parts of the scope it is given', () => {
		expect(scopedHref('/domains/9', { organizationId: 4 })).toBe('/domains/9?org=4');
		expect(scopedHref('/alerts', { organizationId: 4, domainId: 9 })).toBe('/alerts?org=4&domain=9');
	});

	it('is the bare path without them', () => {
		expect(scopedHref('/domains/9', { organizationId: null })).toBe('/domains/9');
		expect(scopedHref('/domains/9', {})).toBe('/domains/9');
	});
});

describe('writeScope', () => {
	it('omits every default', () => {
		expect(writeScope(q(''), DEFAULT_SCOPE, ALL).toString()).toBe('');
		expect(writeScope(q('org=3&since=7d'), DEFAULT_SCOPE, ALL).toString()).toBe('');
	});

	it('writes what differs from the default', () => {
		expect(writeScope(q(''), scope({ organizationId: 3, since: '7d' }), ALL).toString()).toBe('org=3&since=7d');
		expect(writeScope(q(''), scope({ domainId: 12, since: '90d' }), ALL).toString()).toBe('domain=12&since=90d');
	});

	it('writes only what the page carries and keeps other parameters', () => {
		const s = scope({ organizationId: 3, domainId: 12, since: '7d' });
		expect(writeScope(q('tab=senders'), s, ['since']).toString()).toBe('tab=senders&since=7d');
		expect(writeScope(q('domain=example.com'), s, []).toString()).toBe('domain=example.com');
	});

	it('round-trips through readScope', () => {
		const s = scope({ organizationId: 3, domainId: 12, since: '90d' });
		expect(adoptScope(DEFAULT_SCOPE, writeScope(q(''), s, ALL), ALL, 'reset')).toEqual(s);
	});
});

describe('sincePhrase', () => {
	it('reads the range inside a sentence', () => {
		expect(sincePhrase('30d')).toBe('in the last 30 days');
		expect(sincePhrase('7d')).toBe('in the last 7 days');
		expect(sincePhrase(null)).toBe('in all reports');
	});
});
