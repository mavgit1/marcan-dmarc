// What a running mailbox pass says it is doing.
import { describe, expect, it } from 'vitest';
import type { ImapProgressResponse } from '$lib/types/ImapProgressResponse';
import type { SystemHealthResponse } from '$lib/types/SystemHealthResponse';
import { importActivityLabel, importSummary, progressLabel } from './imap-progress';

const progress = (over: Partial<ImapProgressResponse>): ImapProgressResponse =>
	({
		phase: 'importing',
		total_messages: 2950,
		processed_messages: 1240,
		total_bytes: 0,
		processed_bytes: 0,
		...over
	}) as ImapProgressResponse;

describe('progressLabel', () => {
	it('counts the import in messages', () => {
		expect(progressLabel(progress({}))).toBe('Importing 1,240 of 2,950 messages');
	});

	it('says what a scan found, or that it is still looking', () => {
		expect(progressLabel(progress({ phase: 'scanning' }))).toBe('Found 2,950 new messages…');
		expect(progressLabel(progress({ phase: 'scanning', total_messages: 0 }))).toBe('Checking for new mail…');
	});
});

describe('importActivityLabel', () => {
	it('counts the reports stored so far while importing, else names the phase', () => {
		expect(importActivityLabel(progress({ reports_ingested: 1 }))).toBe(
			'Importing the mailbox: 1 report so far (1,240 of 2,950 messages read)'
		);
		expect(importActivityLabel(progress({ phase: 'connecting' }))).toBe('Connecting to the mailbox…');
	});
});

describe('importSummary', () => {
	const health = (running: boolean, reports: number, checking = false) =>
		({
			imap_progress: running ? progress({}) : null,
			total_reports: reports,
			dns_checks: { domains: 14, checked: 3, running: checking, checking }
		}) as SystemHealthResponse;

	it('says the counts are so far while the pass runs, and when DNS is being checked', () => {
		expect(importSummary(health(true, 1204, true))).toBe(
			'Importing: 1,204 reports from 14 domains so far, checking DNS'
		);
		expect(importSummary(health(true, 0))).toBe('Importing: 0 reports from 14 domains so far');
	});

	it('states the counts once it ended, and nothing before any report', () => {
		expect(importSummary(health(false, 1204))).toBe('Imported: 1,204 reports from 14 domains');
		expect(importSummary(health(false, 0))).toBeNull();
	});
});
