// The one query serializer behind every scope, paging, filter and export request.
import { describe, expect, it } from 'vitest';
import { exportCsvUrl } from './api';
import { searchParams } from './http';

describe('searchParams', () => {
	it('drops what carries nothing: null, undefined, empty and false', () => {
		expect(
			searchParams({ organization_id: null, domain_id: undefined, since: '', refresh: false, q: '   ' })
		).toBeUndefined();
		expect(searchParams(undefined)).toBeUndefined();
	});

	it('keeps zero, trims strings and sends true as "true"', () => {
		expect(searchParams({ offset: 0, limit: 50, q: ' acme ', refresh: true, sort: '-count' })).toEqual({
			offset: '0',
			limit: '50',
			q: 'acme',
			refresh: 'true',
			sort: '-count'
		});
	});

	it('serializes a scope the way every list endpoint reads it', () => {
		expect(searchParams({ organization_id: 3, domain_id: null, since: '30d' })).toEqual({
			organization_id: '3',
			since: '30d'
		});
	});
});

describe('exportCsvUrl', () => {
	it('sends only the parameters the dataset reads', () => {
		expect(exportCsvUrl('domains', { organization_id: 2, domain_id: 9, since: '7d', outcome: ['delivered'] })).toBe(
			'/api/export/domains.csv?organization_id=2&since=7d'
		);
		expect(exportCsvUrl('dns-findings', { organization_id: 2, since: '7d' })).toBe(
			'/api/export/dns-findings.csv?organization_id=2'
		);
	});

	it('has no query at all for an unscoped export, and trims the IP filter', () => {
		expect(exportCsvUrl('senders')).toBe('/api/export/senders.csv');
		expect(exportCsvUrl('auth-issues', { domain_id: 1, source_ip: ' 203.0.113.9 ' })).toBe(
			'/api/export/auth-issues.csv?domain_id=1&source_ip=203.0.113.9'
		);
	});

	it('sends the ticked kinds as one comma-separated outcome, none when empty', () => {
		expect(exportCsvUrl('auth-issues', { domain_id: 1, outcome: ['blocked', 'dkim_only'] })).toBe(
			'/api/export/auth-issues.csv?domain_id=1&outcome=blocked%2Cdkim_only'
		);
		expect(exportCsvUrl('auth-issues', { domain_id: 1, outcome: [] })).toBe('/api/export/auth-issues.csv?domain_id=1');
	});
});
