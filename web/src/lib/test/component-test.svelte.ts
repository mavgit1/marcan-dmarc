import { cleanup, render, type RenderResult } from '@testing-library/svelte';
import type { Component } from 'svelte';
import { afterEach, vi } from 'vitest';
import type { Feature } from '$lib/components/settings/settings-form.svelte';
import type { SettingsPageId } from '$lib/settings-nav';
import SettingsPageHarness from './settings-page-harness.svelte';

/**
 * The component tests' shared steps (`*.svelte.test.ts` beside a component, in happy-dom):
 * unmount after each test, every request left pending (a component's loads show their skeleton),
 * and a page around a form.
 */

afterEach(() => {
	cleanup();
	vi.unstubAllGlobals();
});

/** No request answers: `fetch` stays pending, so nothing lands mid-test. */
export function pendingFetch(): void {
	vi.stubGlobal(
		'fetch',
		vi.fn(() => new Promise<Response>(() => {}))
	);
}

/** Let promises settle and Svelte flush its effects (a macrotask later). */
export const settle = () => new Promise<void>((done) => setTimeout(done));

/** Run `make` where a model may own effects (as a component's initialisation); `stop` ends them. */
export function owned<T>(make: () => T): { value: T; stop: () => void } {
	let value!: T;
	const stop = $effect.root(() => {
		value = make();
	});
	return { value, stop };
}

/** Render `form` with `props` inside a Settings page (its form, switch and save bar). */
export async function renderPage<P extends Record<string, unknown>>(
	form: Component<P>,
	props: P,
	options: { page?: SettingsPageId; feature?: () => Feature | null } = {}
): Promise<RenderResult<typeof SettingsPageHarness>> {
	const result = render(SettingsPageHarness, { props: { form, props, ...options } as never });
	await settle();
	return result;
}

/** The save bar's status line and its Save button. */
export function saveBar(container: HTMLElement): { status: HTMLElement | null; save: HTMLButtonElement } {
	const save = [...container.querySelectorAll('button')].find((b) => b.textContent?.trim() === 'Save');
	if (!save) throw new Error('No Save button: the save bar is hidden.');
	const bar = save.parentElement!;
	return { status: bar.querySelector('p'), save };
}
