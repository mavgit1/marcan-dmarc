// @vitest-environment happy-dom
// The Sending page as it renders: the fields from the model, the rows disabled while the switch is
// off, and a disabled Save that says why and marks the field.
import { fireEvent } from '@testing-library/svelte';
import { beforeEach, describe, expect, it } from 'vitest';
import { owned, pendingFetch, renderPage, saveBar, settle } from '$lib/test/component-test.svelte';
import { CONNECTED, settingsFixture } from '$lib/test/settings-fixture';
import type { SettingsResponse } from '$lib/types/SettingsResponse';
import { SendingModel } from './sending-draft.svelte';
import SendingSettingsForm from './sending-settings-form.svelte';

beforeEach(pendingFetch);

async function page(saved: SettingsResponse) {
	const model = owned(() => new SendingModel({ source: { value: () => saved } }));
	const m = model.value;
	const view = await renderPage(SendingSettingsForm, { model: m }, { feature: () => m.feature });
	return {
		m,
		view,
		stop: model.stop,
		input: (id: string) => view.container.querySelector<HTMLInputElement>(`#${id}`)!
	};
}

describe('SendingSettingsForm', () => {
	it('shows the saved account in its fields, and no save bar while nothing changed', async () => {
		const { view, input, stop } = await page(
			settingsFixture(CONNECTED, {
				sending: { enabled: true, host: 'smtp.example.net', port: 587, security: 'starttls', sender: 'a@example.net' }
			})
		);
		expect(input('smtp-host').value).toBe('smtp.example.net');
		expect(input('smtp-port').value).toBe('587');
		expect(input('smtp-sender').value).toBe('a@example.net');
		expect(input('sending-enabled').checked).toBe(true);
		expect(view.queryByRole('button', { name: 'Save' })).toBeNull();
		stop();
	});

	it('disables the rows below the switch while it is off, and keeps their values', async () => {
		const { input, stop } = await page(settingsFixture(CONNECTED, { sending: { host: 'smtp.example.net' } }));
		const rows = input('smtp-host').closest('fieldset')!;
		expect(rows.disabled).toBe(true);
		expect(input('smtp-host').value).toBe('smtp.example.net');
		// The switch itself is outside the disabled rows.
		expect(input('sending-enabled').closest('fieldset')).toBeNull();
		await fireEvent.click(input('sending-enabled'));
		expect(rows.disabled).toBe(false);
		stop();
	});

	it('switched on without a server: Save is disabled, says why in the warn tone and marks the host', async () => {
		const { view, input, stop } = await page(settingsFixture(CONNECTED));
		await fireEvent.click(input('sending-enabled'));
		const { status, save } = saveBar(view.container);
		expect(save.disabled).toBe(true);
		expect(status?.textContent?.trim()).toBe('Enter a server first.');
		expect(status?.querySelector('span')?.className).toContain('text-status-warn-fg');
		expect(input('smtp-host').getAttribute('aria-invalid')).toBe('true');

		// A typed then cleared port holds nothing: the server takes the usual one.
		await fireEvent.input(input('smtp-port'), { target: { value: '2525' } });
		await fireEvent.input(input('smtp-port'), { target: { value: '' } });
		expect(saveBar(view.container).status?.textContent?.trim()).toBe('Enter a server first.');

		await fireEvent.input(input('smtp-host'), { target: { value: 'smtp.example.com' } });
		await settle();
		expect(saveBar(view.container).save.disabled).toBe(false);
		expect(saveBar(view.container).status?.textContent?.trim()).toBe('Unsaved changes');
		expect(input('smtp-host').hasAttribute('aria-invalid')).toBe(false);
		stop();
	});
});
