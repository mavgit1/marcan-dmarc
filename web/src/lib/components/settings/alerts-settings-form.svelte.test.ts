// @vitest-environment happy-dom
// The Alerts page as it renders: the saved recipients and digest, the rows disabled while the
// switch is off, and Save held with its reason while switched on without a recipient.
import { fireEvent } from '@testing-library/svelte';
import { beforeEach, describe, expect, it } from 'vitest';
import { owned, pendingFetch, renderPage, saveBar } from '$lib/test/component-test.svelte';
import { CONNECTED, settingsFixture } from '$lib/test/settings-fixture';
import type { SettingsResponse } from '$lib/types/SettingsResponse';
import { AlertsModel } from './alerts-draft.svelte';
import AlertsSettingsForm from './alerts-settings-form.svelte';

beforeEach(pendingFetch);

async function page(saved: SettingsResponse) {
	const model = owned(() => new AlertsModel({ source: { value: () => saved } }));
	const m = model.value;
	const view = await renderPage(AlertsSettingsForm, { model: m }, { page: 'alerts', feature: () => m.feature });
	const field = <E extends HTMLElement>(id: string) => view.container.querySelector<E>(`#${id}`)!;
	return { view, field, stop: model.stop };
}

describe('AlertsSettingsForm', () => {
	it('shows the saved recipients one per line and the digest', async () => {
		const { field, stop } = await page(
			settingsFixture(CONNECTED, {
				alerts: { enabled: true, recipients: ['a@example.com', 'b@example.com'], digest: 'weekly' }
			})
		);
		expect(field<HTMLTextAreaElement>('alerts-recipients').value).toBe('a@example.com\nb@example.com');
		expect(field<HTMLSelectElement>('alerts-digest').value).toBe('weekly');
		expect(field<HTMLSelectElement>('alerts-weekday').value).toBe('monday');
		expect(field<HTMLInputElement>('alerts-enabled').checked).toBe(true);
		stop();
	});

	it('disables the rows while the switch is off', async () => {
		const { field, stop } = await page(settingsFixture(CONNECTED, { alerts: { recipients: ['a@example.com'] } }));
		expect(field('alerts-recipients').closest('fieldset')!.disabled).toBe(true);
		await fireEvent.click(field('alerts-enabled'));
		expect(field('alerts-recipients').closest('fieldset')!.disabled).toBe(false);
		stop();
	});

	it('switched on without a recipient: Save says why and marks the recipients', async () => {
		const { view, field, stop } = await page(settingsFixture(CONNECTED));
		await fireEvent.click(field('alerts-enabled'));
		const { status, save } = saveBar(view.container);
		expect(save.disabled).toBe(true);
		expect(status?.textContent?.trim()).toBe('Add at least one recipient first.');
		expect(status?.querySelector('span')?.className).toContain('text-status-warn-fg');
		expect(field('alerts-recipients').getAttribute('aria-invalid')).toBe('true');

		await fireEvent.input(field('alerts-recipients'), { target: { value: 'nope' } });
		// A typed address the server would refuse is said under the field; Save sends it and shows the refusal.
		expect(view.getByText('nope is not a valid email address.')).toBeTruthy();
		expect(saveBar(view.container).save.disabled).toBe(false);
		expect(field('alerts-recipients').hasAttribute('aria-invalid')).toBe(false);
		stop();
	});
});
