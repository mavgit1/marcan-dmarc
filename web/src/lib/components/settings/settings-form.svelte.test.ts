// The one base of every settings model: what counts as dirty, what a refresh may overwrite, what
// save and discard do to the draft, what holds Save and the test, and how unsaved edits hold a
// navigation.
import { describe, expect, it, vi } from 'vitest';
import { holdForUnsaved, SettingsForm, unsavedSettings, type SharedSource } from './settings-form.svelte';

type Saved = { text: string; defaults: string[] };
type Draft = { enabled: boolean; text: string; password: string };

/** Let promises settle and Svelte flush its effects (a macrotask later). */
const settle = () => new Promise<void>((done) => setTimeout(done));

type Options = {
	value?: () => Saved | null;
	load?: () => Promise<Saved>;
	shared?: SharedSource<Saved>;
	save?: (d: Draft, s: Saved) => Promise<{ saved: Saved; message?: string }>;
	saveError?: (e: unknown) => string;
};

/** A model as small as a settings page's: the text trimmed, a password, a switch. */
class TextForm extends SettingsForm<Saved, Draft> {
	#save: Options['save'];
	constructor(options: Options) {
		super({
			...(options.shared
				? { shared: options.shared }
				: options.load
					? { key: 'test', load: options.load }
					: { value: options.value ?? (() => null) }),
			label: 'Text',
			blank: { enabled: false, text: '', password: '' },
			secret: 'password',
			feature: { title: 'Use text', hint: 'Turns the text on.', off: 'Turn on the text to test it.' },
			saveError: options.saveError
		});
		this.#save = options.save;
	}
	protected toDraft(s: Saved): Draft {
		return { enabled: s.text !== '', text: s.text, password: '' };
	}
	protected normalize(d: Draft): Draft {
		return { ...d, text: d.text.trim() };
	}
	protected check(d: Draft): string | null {
		return d.text.includes('!') ? 'No exclamation marks.' : null;
	}
	protected whyBlocked(d: Draft): string | null {
		return d.enabled && !d.text ? 'Enter a text first.' : null;
	}
	protected whyNoTest(d: Draft): string | null {
		return d.text === 'untestable' ? 'Not this one.' : null;
	}
	protected write(d: Draft, s: Saved) {
		return this.#save ? this.#save(d, s) : Promise.resolve({ saved: { ...s, text: d.text } });
	}
	sendTest() {
		return this.runTest(async () => ({ ok: true, message: `Tested ${this.normalized.text}.` }));
	}
}

async function setup(options: Options) {
	let form!: TextForm;
	const stop = $effect.root(() => {
		form = new TextForm(options);
	});
	await settle();
	return { form, stop };
}

describe('SettingsForm', () => {
	it('is not dirty before a value, nor when the draft equals the saved fields', async () => {
		let value = $state<Saved | null>(null);
		const { form, stop } = await setup({ value: () => value });
		expect(form.saved).toBeNull();
		expect(form.dirty).toBe(false);
		value = { text: 'a\nb', defaults: [] };
		await settle();
		expect(form.saved).toEqual(value);
		expect(form.draft).toEqual({ enabled: true, text: 'a\nb', password: '' });
		expect(form.dirty).toBe(false);
		stop();
	});

	it('compares the draft as it would be saved; a typed secret is a change and never compared', async () => {
		const { form, stop } = await setup({ value: () => ({ text: 'a', defaults: [] }) });
		form.draft.text = ' a ';
		expect(form.normalized.text).toBe('a');
		expect(form.dirty).toBe(false);
		form.draft.password = 'hunter2';
		expect(form.dirty).toBe(true);
		form.draft.password = '';
		expect(form.dirty).toBe(false);
		stop();
	});

	it('holds Save with its problem, blocks the page with its reason, and the test with the switch', async () => {
		const { form, stop } = await setup({ value: () => ({ text: '', defaults: [] }) });
		expect(form.feature).toMatchObject({ title: 'Use text', on: false });
		expect(form.testBlocked).toBe('Turn on the text to test it.');
		form.feature!.set(true);
		expect(form.draft.enabled).toBe(true);
		expect(form.blocked).toBe('Enter a text first.');
		expect(form.testBlocked).toBeNull();
		form.draft.text = 'hi!';
		expect(form.problem).toBe('No exclamation marks.');
		expect(form.canSave).toBe(false);
		expect(form.blocked).toBeNull();
		form.draft.text = 'untestable';
		expect(form.canSave).toBe(true);
		expect(form.testBlocked).toBe('Not this one.');
		stop();
	});

	it('a save clears the last test line', async () => {
		const { form, stop } = await setup({ value: () => ({ text: 'a', defaults: [] }) });
		await form.sendTest();
		expect(form.test.feedback).toEqual({ ok: true, message: 'Tested a.' });
		form.draft.text = 'b';
		await form.persist();
		expect(form.test.feedback).toBeNull();
		stop();
	});

	it('becomes dirty on an edit, registers as unsaved, and discard() takes it back', async () => {
		const { form, stop } = await setup({ value: () => ({ text: 'a', defaults: [] }) });
		form.draft.text = 'a\nb';
		await settle();
		expect(form.dirty).toBe(true);
		expect(unsavedSettings.any).toBe(true);
		form.discard();
		await settle();
		expect(form.draft.text).toBe('a');
		expect(form.dirty).toBe(false);
		expect(unsavedSettings.any).toBe(false);
		stop();
	});

	it('a new value replaces an unedited draft but keeps an edited one', async () => {
		let value = $state<Saved | null>({ text: 'a', defaults: [] });
		const { form, stop } = await setup({ value: () => value });
		value = { text: 'b', defaults: [] };
		await settle();
		expect(form.draft.text).toBe('b');
		form.draft.text = 'typed';
		value = { text: 'c', defaults: [] };
		await settle();
		expect(form.draft.text).toBe('typed');
		expect(form.saved?.text).toBe('c');
		expect(form.dirty).toBe(true);
		stop();
	});

	it('submit() saves the normalized draft: the answer is the new saved value, the status line "Saved"', async () => {
		const calls: Draft[] = [];
		const { form, stop } = await setup({
			value: () => ({ text: 'a', defaults: ['x'] }),
			save: async (d, s) => {
				calls.push(d);
				return { saved: { ...s, text: `${d.text} (normalized)` }, message: 'Saved: 1 added' };
			}
		});
		form.draft.text = ' a\nb ';
		form.submit();
		await settle();
		expect(calls).toEqual([{ enabled: true, text: 'a\nb', password: '' }]);
		expect(form.saved).toEqual({ text: 'a\nb (normalized)', defaults: ['x'] });
		expect(form.draft.text).toBe('a\nb (normalized)');
		expect(form.dirty).toBe(false);
		expect(form.save.feedback).toEqual({ ok: true, message: 'Saved: 1 added' });
		stop();
	});

	it('a rejected save keeps the draft and words the failure', async () => {
		const { form, stop } = await setup({
			value: () => ({ text: 'a', defaults: [] }),
			save: async () => {
				throw new Error('nope');
			},
			saveError: (e) => `Could not save: ${(e as Error).message}`
		});
		form.draft.text = 'b';
		form.submit();
		await settle();
		expect(form.draft.text).toBe('b');
		expect(form.dirty).toBe(true);
		expect(form.save.feedback).toEqual({ ok: false, message: 'Could not save: nope' });
		stop();
	});

	it('loads its own value with a loader, and reports a failed first load', async () => {
		const { form, stop } = await setup({ load: () => Promise.resolve({ text: 'loaded', defaults: [] }) });
		expect(form.saved?.text).toBe('loaded');
		expect(form.loadError).toBeNull();
		stop();
		const failed = await setup({ load: () => Promise.reject(new Error('offline')) });
		expect(failed.form.saved).toBeNull();
		expect((failed.form.loadError as Error).message).toBe('offline');
		failed.stop();
	});

	it('reads a shared copy: asks it to load, follows its changes, and retries through it', async () => {
		let loads = 0;
		let reloads = 0;
		const shared = new (class implements SharedSource<Saved> {
			current = $state.raw<Saved | null>(null);
			error = $state.raw<unknown>(new Error('offline'));
			async load() {
				loads += 1;
			}
			async reload() {
				reloads += 1;
			}
		})();
		const one = await setup({ shared });
		const two = await setup({ shared });
		expect(loads).toBe(2);
		expect((one.form.loadError as Error).message).toBe('offline');
		one.form.reload();
		expect(reloads).toBe(1);

		shared.error = null;
		shared.current = { text: 'a', defaults: [] };
		await settle();
		expect(one.form.loadError).toBeNull();
		expect(one.form.draft.text).toBe('a');
		two.form.draft.text = 'typed';
		// Another section saved: every form sees the new value, an edited draft stays.
		shared.current = { text: 'b', defaults: [] };
		await settle();
		expect(one.form.draft.text).toBe('b');
		expect(two.form.saved?.text).toBe('b');
		expect(two.form.draft.text).toBe('typed');
		one.stop();
		two.stop();
	});

	// The page used `window.confirm`, which an embedded browser answers with `false` unseen:
	// the navigation was cancelled and nothing on the page said why.
	describe('holding a navigation for unsaved edits', () => {
		const nav = (to: string, willUnload = false) => {
			const cancel = vi.fn();
			return {
				nav: {
					from: { url: new URL('http://x/settings/alerts') },
					to: { url: new URL(`http://x${to}`) },
					willUnload,
					cancel
				},
				cancel
			};
		};

		it('lets every navigation through while nothing is unsaved', async () => {
			const { stop } = await setup({ value: () => ({ text: 'a', defaults: [] }) });
			const { nav: n, cancel } = nav('/settings/data');
			holdForUnsaved(n);
			expect(cancel).not.toHaveBeenCalled();
			stop();
		});

		it('holds another page with unsaved edits and says so; after Discard it goes', async () => {
			const confirm = vi.fn(() => false);
			vi.stubGlobal('confirm', confirm);
			const { form, stop } = await setup({ value: () => ({ text: 'a', defaults: [] }) });
			form.draft.text = 'b';
			await settle();
			const before = unsavedSettings.held;
			for (const to of ['/settings/data', '/domains']) {
				const { nav: n, cancel } = nav(to);
				holdForUnsaved(n);
				expect(cancel).toHaveBeenCalledTimes(1);
			}
			// Each held navigation is announced to the save bars, never asked in a hidden dialog.
			expect(unsavedSettings.held).toBe(before + 2);
			expect(confirm).not.toHaveBeenCalled();

			// A section's anchor on the same page is not a departure.
			const same = nav('/settings/alerts#alert-kinds');
			holdForUnsaved(same.nav);
			expect(same.cancel).not.toHaveBeenCalled();

			form.discard();
			await settle();
			const again = nav('/settings/data');
			holdForUnsaved(again.nav);
			expect(again.cancel).not.toHaveBeenCalled();
			expect(unsavedSettings.held).toBe(before + 2);
			stop();
			vi.unstubAllGlobals();
		});

		it('leaves closing or reloading the browser tab to the browser', async () => {
			const { form, stop } = await setup({ value: () => ({ text: 'a', defaults: [] }) });
			form.draft.text = 'b';
			await settle();
			const before = unsavedSettings.held;
			const { nav: n, cancel } = nav('/', true);
			holdForUnsaved(n);
			expect(cancel).toHaveBeenCalledTimes(1);
			expect(unsavedSettings.held).toBe(before);
			stop();
		});
	});
});
