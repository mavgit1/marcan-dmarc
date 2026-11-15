// The pipeline's labels while an import is unfinished.
import { describe, expect, it } from 'vitest';
import type { PassProgressResponse } from '$lib/types/PassProgressResponse';
import type { Pipeline } from '$lib/types/Pipeline';
import type { SystemHealthResponse } from '$lib/types/SystemHealthResponse';
import { pipelineStatus } from './pipeline';

const importing: Pipeline = {
	mailbox: 'importing',
	mailbox_status: 'pass',
	reports: 'importing',
	reports_status: 'pass',
	status: 'pass',
	headline: 'importing'
};

const health = (pipeline: Pipeline, pass_progress: Partial<PassProgressResponse> | null): SystemHealthResponse =>
	({
		pipeline,
		pass_progress: pass_progress && ({ phase: 'importing', ...pass_progress } as PassProgressResponse),
		last_check_at: null,
		last_ingest_at: null
	}) as SystemHealthResponse;

describe('pipelineStatus while importing', () => {
	it('names the import between the passes of a long import, without a count', () => {
		const s = pipelineStatus(health(importing, null));
		expect(s.overall.label).toBe('Importing');
		expect(s.reports.label).toBe('Importing');
		expect(s.mailbox.label).toBe('Importing');
	});

	it('counts the messages left while a pass downloads', () => {
		const s = pipelineStatus(health(importing, { total_messages: 300, processed_messages: 142 }));
		expect(s.reports.label).toBe('Importing, 158 messages left');
		expect(s.overall.label).toBe('Importing 142 of 300 messages');
	});

	it('reads up to date only once the import is done', () => {
		const done: Pipeline = {
			...importing,
			mailbox: 'active',
			reports: 'up_to_date',
			headline: 'all_well'
		};
		const s = pipelineStatus(health(done, null));
		expect(s.overall.label).toBe('Up to date');
		expect(s.reports.label).toBe('Up to date');
	});
});
