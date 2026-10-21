// @vitest-environment happy-dom
// The save bar on its own: hidden with nothing to save, Save disabled only with a reason that
// stands beside it in the warn tone, and a refused save's sentence kept above that reason.
import { render } from '@testing-library/svelte';
import { describe, expect, it } from 'vitest';
import { saveBar } from '$lib/test/component-test.svelte';
import { Action } from '../action.svelte';
import SaveBar from './save-bar.svelte';

const props = (over: { dirty?: boolean; reason?: string | null; save?: Action } = {}) => ({
	dirty: true,
	save: new Action(),
	reason: null,
	onDiscard: () => {},
	...over
});

/** The status line's parts: each sentence and its tone class. */
const parts = (status: HTMLElement | null) =>
	[...(status?.querySelectorAll(':scope > span') ?? [])].map((s) => [s.textContent?.trim(), s.className]);

describe('SaveBar', () => {
	it('is hidden while there is nothing to save', () => {
		const { container } = render(SaveBar, props({ dirty: false, reason: 'Enter a server first.' }));
		expect(container.querySelector('button')).toBeNull();
	});

	it('unsaved and savable: "Unsaved changes", muted, Save enabled', () => {
		const { status, save } = saveBar(render(SaveBar, props()).container);
		expect(parts(status)).toEqual([['Unsaved changes', 'text-muted-foreground']]);
		expect(save.disabled).toBe(false);
	});

	it('a reason disables Save and stands beside it in the warn tone', () => {
		const { status, save } = saveBar(render(SaveBar, props({ reason: 'Keep reports for 1 to 120 months.' })).container);
		expect(save.disabled).toBe(true);
		expect(parts(status)).toEqual([['Keep reports for 1 to 120 months.', 'text-status-warn-fg break-words']]);
	});

	it('after a refused save the refusal stays, with the reason after it', () => {
		const save = new Action();
		save.feedback = { ok: false, message: 'Schedule saved. Recipients: Line 2: no such organization' };
		const bar = saveBar(render(SaveBar, props({ save, reason: 'Fix 1 line before saving.' })).container);
		expect(parts(bar.status)).toEqual([
			['Schedule saved. Recipients: Line 2: no such organization', 'text-status-fail-fg break-words'],
			['Fix 1 line before saving.', 'text-status-warn-fg break-words']
		]);
		expect(bar.save.disabled).toBe(true);
	});
});
