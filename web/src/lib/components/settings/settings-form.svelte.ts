import { untrack } from 'svelte';
import { SvelteSet } from 'svelte/reactivity';
import { apiErrorMessage } from '$lib/http';
import { refresh } from '$lib/state/refresh.svelte';
import { resource, type Resource } from '$lib/state/resource.svelte';
import type { SettingsPageId } from '$lib/settings-nav';
import type { SettingsResponse } from '$lib/types/SettingsResponse';
import { Action, type Feedback } from '../action.svelte';
import type { PagePart } from './page-form.svelte';

/** An app-wide copy of the saved value, loaded once and read by several forms (`settings`). */
export type SharedSource<T> = {
	/** `null` before the first answer. */
	readonly current: T | null;
	/** Why the last load failed. */
	readonly error: unknown;
	/** Fetch unless already fetched. */
	load(): Promise<unknown>;
	/** Fetch again. */
	reload(): Promise<unknown>;
};

/**
 * Where the saved value comes from: loaded by the form itself (a list), an app-wide copy (the
 * settings groups), or handed in by the parent (a setup step, a test).
 */
export type SettingsFormSource<T> =
	| {
			/** Resource key; distinct per section. */
			key: string;
			load: () => Promise<T>;
	  }
	| {
			/** The app-wide copy; saves update it (`settings.save`), so every reader sees them. */
			shared: SharedSource<T>;
	  }
	| {
			/** The parent's copy of the saved value; `null` before it is known. */
			value: () => T | null;
	  };

/** The words of a feature page's switch and of its test or preview. */
export type FeatureWords = {
	/** The switch row's title: "Send alerts". */
	title: string;
	/** One line: what the switch turns on. */
	hint: string;
	/** Why the test waits while the switch is off: "Turn on alert emails to send a test.". */
	off: string;
};

/**
 * A feature page's switch (Sending, Alerts, Client reports): the first row of the page's form,
 * the draft's `enabled`, saved by the save bar like every other field. While it is off, the rows
 * below it are disabled and keep their values.
 */
export type Feature = {
	readonly title: string;
	readonly hint: string;
	/** On, in the draft. */
	readonly on: boolean;
	/** Flip the draft's switch. */
	set(on: boolean): void;
};

export type SettingsFormOptions<T, D> = SettingsFormSource<T> & {
	/** What the save bar's status line calls the part when a later one fails ("Schedule saved."). */
	label: string;
	/** The draft before the first value (the form is not shown before it). */
	blank: D;
	/** A field typed to change a stored secret (a password): a change once typed, never compared. */
	secret?: keyof D;
	/** A feature page's switch over the draft's `enabled` (`feature`, `testBlocked`). */
	feature?: FeatureWords;
	/** Words a rejected save; default `apiErrorMessage`. */
	saveError?: (error: unknown) => string;
	/** After each value arrives; `first` on the first one. */
	onLoad?: (saved: T, first: boolean) => void;
};

/**
 * What is wrong with a draft: the sentence the save bar shows and the field it is about, marked
 * invalid on the page (a draft key, or a name the page gives a field of another part:
 * Client reports' `recipients`).
 */
export type Invalid = { field: string; message: string };

/** `message` about `field`; `null` without a message. */
export function invalid(field: string, message: string | null): Invalid | null {
	return message === null ? null : { field, message };
}

/** Why Save is disabled when a model names no reason (a safety net: every model names one). */
export const SAVE_HELD = 'Correct the marked fields first.';

/** What every settings page names when a test or preview has nowhere to go (the server says the same). */
export const NO_TEST_RECIPIENT = 'Set a test recipient under Sending first.';

/** Where tests and previews go: the test recipient saved under Sending; empty when nowhere. */
export function savedTestRecipient(saved: SettingsResponse | null): string {
	return saved?.sending.effective_test_recipient ?? '';
}

/**
 * Why a test or preview through the saved sending account cannot go now, worded like the
 * server's refusal (client-side only to hold the button with its reason): nowhere to send it, or
 * sending off as saved. `what` is what the button sends: "test", "preview".
 */
export function savedSendingBlocks(saved: SettingsResponse | null, what: string): string | null {
	if (!savedTestRecipient(saved)) return NO_TEST_RECIPIENT;
	if (!saved?.sending.enabled) return `Turn on sending to send a ${what}.`;
	return null;
}

/** Every form with unsaved edits, so the page can hold a navigation that would lose them. */
const dirtyForms = new SvelteSet<object>();
/** Counts the navigations held for unsaved edits; each one makes the dirty save bars show themselves. */
const held = $state({ count: 0 });

export const unsavedSettings = {
	/** A form on the page has unsaved edits. */
	get any(): boolean {
		return dirtyForms.size > 0;
	},
	/** How many navigations were held so far (`SaveBar` reacts to each new one). */
	get held(): number {
		return held.count;
	}
};

/** What `holdForUnsaved` needs of SvelteKit's `beforeNavigate` argument. */
export type HeldNavigation = {
	from: { url: URL } | null;
	to: { url: URL } | null;
	willUnload: boolean;
	cancel: () => void;
};

/**
 * The Settings layout's `beforeNavigate`: while a form has unsaved edits, a navigation to another
 * page is held and the page's save bar shows itself (save or discard, then go). A link within
 * the same page (a section's anchor) goes through; closing or reloading the browser tab is held
 * for the browser's own question. Nothing is held silently: `window.confirm` is not
 * used, since embedded browsers and "prevent more dialogs" answer it with `false` unseen.
 */
export function holdForUnsaved(nav: HeldNavigation): void {
	if (dirtyForms.size === 0) return;
	if (nav.willUnload) return nav.cancel();
	const from = nav.from?.url;
	const to = nav.to?.url;
	if (from && to && to.pathname === from.pathname && to.search === from.search) return;
	nav.cancel();
	held.count += 1;
}

/** Which Settings page a model belongs to and the settings groups (`PUT /api/settings/{group}`) it writes. */
export type Writes = { readonly page: SettingsPageId; readonly groups: readonly string[] };

/**
 * One part of a Settings page as a form, the one base of every settings model (`ImapModel`,
 * `SendingModel`, `AlertsModel`, `ClientReportsModel`, `LicenseModel`, `BrandingModel`,
 * `RetentionModel`, `ResolversModel`, `UpdatesModel`, `ListDraft`): the saved value, an editable
 * `draft` of it (bind to its fields), the draft as it would be saved (`normalized`), whether the two differ
 * (`dirty`), why it cannot be saved (`problem`, which holds Save) or why the page cannot be
 * (`blocked`), the one reason a disabled Save states (`held`) and the field it marks invalid
 * (`fieldProblem`, `ariaInvalid`), the order it saves in (`saveLast`), save and
 * discard, and one test or preview (`test`, `testBlocked`). A model is itself the part it joins
 * the page form with (`joinPageForm(model)`). The draft survives app refreshes (a reload only
 * replaces an unedited draft); a save replaces the saved value and bumps `refresh`. Create it
 * during component initialisation (it owns effects).
 *
 * Every Settings page makes its models in its route (`+page.svelte`) and hands each to the
 * component that shows it as `model` (or `list`); the setup's forms make their own `ImapModel`.
 * Each model declares its page and groups in `static writes`, which `settings-ownership.test.ts`
 * checks: a group written by two pages would overwrite the other's unsaved values. No section
 * hand-rolls load, dirty or save.
 */
export abstract class SettingsForm<T, D> implements PagePart {
	readonly label: string;
	/** The fields being edited (deeply reactive, bind to them). */
	draft = $state() as D;
	/** The save; its feedback is the status line. */
	readonly save = new Action();
	/** The page's test or preview; its feedback is the result line next to its button, cleared by a save. */
	readonly test = new Action();
	/** The draft as it would be saved. */
	normalized: D = $derived.by(() => this.normalize(this.draft));
	/** What is wrong with the draft and where; `null` when it can be saved. */
	invalid: Invalid | null = $derived.by(() => this.check(this.normalized));
	/** Why the draft cannot be saved; `null` when it can. Save waits while it says so. */
	problem: string | null = $derived(this.invalid?.message ?? null);
	#blockedBy: Invalid | null = $derived.by(() => this.whyBlocked(this.normalized));
	/** Why the page cannot be saved as it is, said beside a disabled Save; `null` when it can. */
	blocked: string | null = $derived(this.#blockedBy?.message ?? null);
	/** Why the test cannot run now (the switch off first); `null` when it can. */
	testBlocked: string | null = $derived.by(() =>
		this.#feature && !(this.normalized as { enabled?: boolean }).enabled
			? this.#feature.off
			: this.whyNoTest(this.normalized)
	);
	#saved = $state.raw<T | null>(null);
	#base = $derived.by(() => (this.#saved === null ? null : this.toDraft(this.#saved)));
	#source: Resource<T> | null = null;
	#shared: SharedSource<T> | null = null;
	#options: SettingsFormOptions<T, D>;
	#feature: FeatureWords | undefined;

	constructor(options: SettingsFormOptions<T, D>) {
		this.#options = options;
		this.#feature = options.feature;
		this.label = options.label;
		this.draft = options.blank;
		if ('load' in options) {
			this.#source = resource({
				key: () => options.key,
				load: options.load,
				onData: (data) => this.#apply(data)
			});
		} else if ('shared' in options) {
			const shared = options.shared;
			this.#shared = shared;
			shared.load().catch(() => {});
			$effect(() => {
				const value = shared.current;
				if (value !== null) untrack(() => this.#apply(value));
			});
		} else {
			$effect(() => {
				const value = options.value();
				if (value !== null) untrack(() => this.#apply(value));
			});
		}
		$effect(() => {
			if (this.dirty) dirtyForms.add(this);
			else dirtyForms.delete(this);
			return () => {
				dirtyForms.delete(this);
			};
		});
	}

	/** The editable fields of the saved value: fresh objects, never `saved` itself. */
	protected abstract toDraft(saved: T): D;

	/** Persist the normalized draft: the new saved value (shown at once) and the status line (default "Saved"). Throw to fail. */
	protected abstract write(draft: D, saved: T): Promise<{ saved: T; message?: string }>;

	/** The draft as it would be saved (trimmed, numbers as numbers); default: as typed. */
	protected normalize(draft: D): D {
		return draft;
	}

	/**
	 * The draft differs from the saved fields: by default the normalized draft, structurally, and a
	 * typed `secret` is a change by itself and never compared (the stored one is never sent back).
	 */
	protected isDirty(draft: D, base: D): boolean {
		const d = { ...draft };
		const s = { ...base };
		const secret = this.#options.secret;
		if (secret !== undefined) {
			const typed = d[secret] !== '';
			delete d[secret];
			delete s[secret];
			if (typed) return true;
		}
		return JSON.stringify(d) !== JSON.stringify(s);
	}

	/** Why a normalized draft cannot be saved, worded like the server's own check, and its field; default none. */
	protected check(_draft: D): Invalid | null {
		return null;
	}

	/** Why the page cannot be saved as it is (a feature on that cannot work), and its field; default none. */
	protected whyBlocked(_draft: D): Invalid | null {
		return null;
	}

	/** Why the switched-on test cannot run now; default none. */
	protected whyNoTest(_draft: D): string | null {
		return null;
	}

	/** The value as last loaded or saved; `null` before the first load. */
	get saved(): T | null {
		return this.#saved;
	}

	/** `toDraft` of the saved value: what the draft is compared with. */
	get base(): D | null {
		return this.#base;
	}

	/** Why nothing could be loaded (only before the first value; later failures keep it). */
	get loadError(): unknown {
		if (this.#saved !== null) return null;
		return this.#source?.error ?? this.#shared?.error ?? null;
	}

	/** The draft differs from the saved value. */
	get dirty(): boolean {
		const base = this.#base;
		return base !== null && this.isDirty(this.normalize($state.snapshot(this.draft) as D), base);
	}

	/** Save may run (the page's Save is disabled otherwise); default: nothing is wrong. */
	get canSave(): boolean {
		return this.problem === null;
	}

	/**
	 * Why this part holds the page's Save, and the field to mark: `blocked`'s reason, else (while
	 * `canSave` is false) the problem; `null` when it holds nothing. A disabled Save always states
	 * it (`SaveBar`), so no Save is disabled silently.
	 */
	get held(): Invalid | null {
		if (this.#blockedBy) return this.#blockedBy;
		if (this.canSave) return null;
		return this.invalid ?? { field: '', message: SAVE_HELD };
	}

	/** The reason Save is held when it is about `field` (said under the field where the page has room); else `null`. */
	fieldProblem(field: string): string | null {
		const held = this.held;
		return held?.field === field ? held.message : null;
	}

	/** `aria-invalid` for `field`'s control: set while Save is held because of it. */
	ariaInvalid(field: string): true | undefined {
		return this.fieldProblem(field) !== null || undefined;
	}

	/** Saved after the other parts of its page (a switch that needs them saved first). */
	get saveLast(): boolean {
		return false;
	}

	/** The switch, the page form's first row; `null` without one or before the value is known. */
	get feature(): Feature | null {
		const words = this.#feature;
		if (!words || this.#saved === null) return null;
		const draft = this.draft as { enabled: boolean };
		return { title: words.title, hint: words.hint, on: draft.enabled, set: (on) => (draft.enabled = on) };
	}

	#apply(saved: T) {
		const first = this.#saved === null;
		// Keep an edited draft across app refreshes.
		const edited = this.dirty;
		this.#saved = saved;
		if (!edited) this.draft = this.toDraft(saved);
		this.#options.onLoad?.(saved, first);
	}

	/** Load the saved value again (after a change made elsewhere). */
	reload(): void {
		this.#source?.reload();
		this.#shared?.reload().catch(() => {});
	}

	/** Back to the saved fields; clears the status line. */
	discard(): void {
		if (this.#saved !== null) this.draft = this.toDraft(this.#saved);
		this.save.feedback = null;
	}

	/**
	 * Save the draft and say how it went: the saved value replaces the draft (and the last test's
	 * line goes), or the draft stays with the worded error. `save.busy` is set meanwhile and
	 * `save.feedback` keeps the result.
	 */
	async persist(): Promise<Feedback> {
		const saved = this.#saved;
		if (saved === null) return { ok: false, message: 'Not loaded yet.' };
		await this.save.run(async () => {
			try {
				const draft = this.normalize($state.snapshot(this.draft) as D);
				if (!this.canSave) throw new Error(this.check(draft)?.message ?? 'Not saved.');
				const result = await this.write(draft, saved);
				this.#saved = result.saved;
				this.draft = this.toDraft(result.saved);
				this.test.feedback = null;
				refresh.refresh();
				return { ok: true, message: result.message ?? 'Saved' };
			} catch (error) {
				return { ok: false, message: (this.#options.saveError ?? apiErrorMessage)(error) };
			}
		});
		return this.save.feedback ?? { ok: false, message: 'Not saved.' };
	}

	/** A form of its own (a setup step): save the draft (a rejected save becomes the status line). */
	submit(event?: SubmitEvent): void {
		event?.preventDefault();
		void this.persist();
	}

	/** Run the test or preview; its answer (`ok`, `message`) is the result line. */
	protected runTest(send: () => Promise<{ ok: boolean; message: string }>): Promise<void> {
		return this.test.run(async () => {
			const res = await send();
			return { ok: res.ok, message: res.message };
		});
	}
}
