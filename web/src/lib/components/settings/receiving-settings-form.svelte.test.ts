// @vitest-environment happy-dom
// The form under the Accounts page's cards: the saved schedule and warnings, each group saved
// only when it changed, and a disabled Save that says why and marks the field.
import { fireEvent } from '@testing-library/svelte';
import { beforeEach, describe, expect, it, vi } from 'vitest';
import { owned, pendingFetch, renderPage, saveBar, settle } from '$lib/test/component-test.svelte';
import { CONNECTED, settingsFixture } from '$lib/test/settings-fixture';
import type { SettingsResponse } from '$lib/types/SettingsResponse';
import { ReceivingModel } from './receiving-draft.svelte';
import ReceivingSettingsForm from './receiving-settings-form.svelte';

vi.mock('$app/paths', () => ({ resolve: (path: string) => path }));

beforeEach(pendingFetch);

async function page(saved: SettingsResponse) {
	const model = owned(() => new ReceivingModel({ source: { value: () => saved } }));
	const m = model.value;
	const view = await renderPage(ReceivingSettingsForm, { model: m }, { page: 'accounts' });
	const minutes = () => view.getByLabelText('Minutes between checks') as HTMLInputElement;
	return { m, view, minutes, stop: model.stop };
}

describe('ReceivingSettingsForm', () => {
	it('shows the saved interval in minutes and the warnings, with no save bar', async () => {
		const { view, minutes, stop } = await page(settingsFixture(CONNECTED, { receiving: { poll_interval_secs: 600 } }));
		expect(minutes().value).toBe('10');
		const warn = view.getByLabelText('Hours without any report before the header warns') as HTMLSelectElement;
		expect(warn.value).toBe('24');
		const titles = [...view.container.querySelectorAll('section > h3')].map((h) => h.textContent?.trim());
		expect(titles).toEqual(['Schedule', 'Report arrival']);
		expect(view.queryByRole('button', { name: 'Save' })).toBeNull();
		stop();
	});

	it('below a minute: Save is disabled, says why and marks the field', async () => {
		const { view, minutes, stop } = await page(settingsFixture(CONNECTED));
		await fireEvent.input(minutes(), { target: { value: '0' } });
		await settle();
		const { status, save } = saveBar(view.container);
		expect(save.disabled).toBe(true);
		expect(status?.textContent?.trim()).toBe('Check the accounts at least every minute.');
		expect(minutes().getAttribute('aria-invalid')).toBe('true');
		await fireEvent.input(minutes(), { target: { value: '15' } });
		await settle();
		expect(saveBar(view.container).save.disabled).toBe(false);
		stop();
	});

	it('saves only the groups that changed', async () => {
		const saved = settingsFixture(CONNECTED);
		const sent: { path: string; body: unknown }[] = [];
		vi.stubGlobal(
			'fetch',
			vi.fn(async (req: Request) => {
				sent.push({ path: new URL(req.url).pathname, body: JSON.parse(await req.clone().text()) });
				return new Response(JSON.stringify(saved), { headers: { 'content-type': 'application/json' } });
			})
		);
		const { m, stop } = await page(saved);
		m.draft.pollMinutes = 2;
		await settle();
		await m.persist();
		expect(sent).toEqual([{ path: '/api/settings/receiving', body: { poll_interval_secs: 120 } }]);
		stop();
	});
});
