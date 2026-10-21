// @vitest-environment happy-dom
// The list editor as it renders: the stored list in its box, what an added or removed line
// would change (the server's preview), each line error linked to its line, and Save held with
// the count while there are any.
import { fireEvent, waitFor } from '@testing-library/svelte';
import { beforeEach, describe, expect, it, vi } from 'vitest';
import { owned, pendingFetch, renderPage, saveBar } from '$lib/test/component-test.svelte';
import type { LineError } from '$lib/types/LineError';
import { ListDraft, type ListSpec } from './list-draft.svelte';
import ListEditor from './list-editor.svelte';

beforeEach(pendingFetch);

type Summary = { errors: LineError[]; added: string[]; removed: string[] };

/** A list of names: a line with a space is refused; the preview compares with `stored`. */
function spec(stored: string): ListSpec<Summary> {
	const lines = (text: string) => text.split('\n').filter(Boolean);
	const summary = (text: string): Summary => ({
		errors: lines(text).flatMap((l) =>
			l.includes(' ') ? [{ line: text.split('\n').indexOf(l) + 1, message: `"${l}" has a space` }] : []
		),
		added: lines(text).filter((l) => !lines(stored).includes(l)),
		removed: lines(stored).filter((l) => !lines(text).includes(l))
	});
	return {
		label: 'Names',
		key: 'test-names',
		get: vi.fn(async () => ({ text: stored })),
		preview: vi.fn(async (text: string) => summary(text)),
		save: vi.fn(async (text: string) => summary(text)),
		changes: (s) => ({
			summary: [s.added.length && `${s.added.length} added`, s.removed.length && `${s.removed.length} removed`]
				.filter(Boolean)
				.join(', '),
			details: [
				{ label: 'Added', items: s.added },
				{ label: 'Removed', items: s.removed }
			]
		})
	};
}

async function page(stored: string) {
	const list = owned(() => new ListDraft(spec(stored)));
	const view = await renderPage(ListEditor<Summary>, { list: list.value, title: 'Names' }, { page: 'sender-names' });
	await waitFor(() => expect(view.getByRole('textbox', { name: 'Names' })).toBeTruthy());
	const box = view.getByRole('textbox', { name: 'Names' }) as HTMLTextAreaElement;
	return { view, box, stop: list.stop };
}

describe('ListEditor', () => {
	it('shows the stored list and no save bar', async () => {
		const { view, box, stop } = await page('alpha\nbeta');
		expect(box.value).toBe('alpha\nbeta');
		expect(view.queryByRole('button', { name: 'Save' })).toBeNull();
		stop();
	});

	it('an added line: what saving changes', async () => {
		const { view, box, stop } = await page('alpha');
		await fireEvent.input(box, { target: { value: 'alpha\ngamma' } });
		await waitFor(() => expect(view.getByText('Saving will change: 1 added')).toBeTruthy());
		expect(saveBar(view.container).save.disabled).toBe(false);
		stop();
	});

	it('a removed line: what saving changes', async () => {
		const { view, box, stop } = await page('alpha\nbeta');
		await fireEvent.input(box, { target: { value: 'alpha' } });
		await waitFor(() => expect(view.getByText('Saving will change: 1 removed')).toBeTruthy());
		stop();
	});

	it('a line error: listed under the box with its line, the box marked, Save held with the count', async () => {
		const { view, box, stop } = await page('alpha');
		await fireEvent.input(box, { target: { value: 'alpha\nbad name' } });
		const alert = await waitFor(() => view.getByRole('alert'));
		expect(alert.textContent).toContain('Fix 1 line before saving:');
		expect(view.getByRole('button', { name: 'Line 2' })).toBeTruthy();
		expect(alert.textContent).toContain('"bad name" has a space');
		expect(box.getAttribute('aria-invalid')).toBe('true');
		const { status, save } = saveBar(view.container);
		expect(save.disabled).toBe(true);
		expect(status?.textContent?.trim()).toBe('Fix 1 line before saving.');
		expect(status?.querySelector('span')?.className).toContain('text-status-warn-fg');
		stop();
	});

	it('marked by another part of the page: the reason under the box', async () => {
		const list = owned(() => new ListDraft(spec('')));
		const view = await renderPage(
			ListEditor<Summary>,
			{ list: list.value, title: 'Names', invalid: 'Add at least one recipient to the list first.' },
			{ page: 'client-reports' }
		);
		const box = await waitFor(() => view.getByRole('textbox', { name: 'Names' }));
		expect(box.getAttribute('aria-invalid')).toBe('true');
		expect(view.getByText('Add at least one recipient to the list first.')).toBeTruthy();
		list.stop();
	});
});
