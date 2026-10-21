// @vitest-environment happy-dom
// The Mailbox page's form as it renders: the saved mailbox in its fields, the stored password
// locked, and a save that says what is missing (it is never held: a save logs in first).
import { fireEvent } from '@testing-library/svelte';
import { beforeEach, describe, expect, it, vi } from 'vitest';
import { owned, pendingFetch, renderPage, saveBar, settle } from '$lib/test/component-test.svelte';
import { CONNECTED, settingsFixture } from '$lib/test/settings-fixture';
import { ImapModel } from './imap-draft.svelte';
import ImapSettingsForm from './imap-settings-form.svelte';

beforeEach(pendingFetch);

async function page() {
	const saved = settingsFixture(CONNECTED);
	const write = vi.fn();
	const model = owned(
		(): ImapModel =>
			new ImapModel({
				source: { value: () => saved },
				save: (d, s) => {
					write(d);
					return model.value.saveSettings(d, s);
				}
			})
	);
	const view = await renderPage(ImapSettingsForm, { model: model.value }, { page: 'mailbox' });
	const input = (id: string) => view.container.querySelector<HTMLInputElement>(`#${id}`)!;
	return { view, input, write, stop: model.stop };
}

describe('ImapSettingsForm', () => {
	it('shows the saved mailbox, its stored password locked, and no switch', async () => {
		const { view, input, stop } = await page();
		expect(input('imap-host').value).toBe('imap.example.com');
		expect(input('imap-port').value).toBe('993');
		expect(input('imap-username').value).toBe('dmarc@example.com');
		expect(input('imap-password').value).toBe('Stored');
		expect(view.getByRole('button', { name: 'Replace' })).toBeTruthy();
		expect(view.container.querySelector('[role="switch"]')).toBeNull();
		expect(view.queryByRole('button', { name: 'Save' })).toBeNull();
		stop();
	});

	it('an edit shows the save bar; Discard takes it back', async () => {
		const { view, input, stop } = await page();
		await fireEvent.input(input('imap-host'), { target: { value: 'imap.example.net' } });
		const { status, save } = saveBar(view.container);
		expect(status?.textContent?.trim()).toBe('Unsaved changes');
		expect(save.disabled).toBe(false);
		await fireEvent.click(view.getByRole('button', { name: 'Discard' }));
		await settle();
		expect(input('imap-host').value).toBe('imap.example.com');
		stop();
	});

	it('a save with the host cleared says what is missing', async () => {
		const { view, input, write, stop } = await page();
		await fireEvent.input(input('imap-host'), { target: { value: '' } });
		await fireEvent.click(saveBar(view.container).save);
		await settle();
		expect(write).toHaveBeenCalledOnce();
		const { status } = saveBar(view.container);
		expect(status?.textContent?.trim()).toBe('Enter the server host.');
		expect(status?.querySelector('span')?.className).toContain('text-status-fail-fg');
		stop();
	});
});
