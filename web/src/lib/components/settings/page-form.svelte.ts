import { getContext, onDestroy, setContext } from 'svelte';
import { Action, type Feedback } from '../action.svelte';
import { SAVE_HELD } from './settings-form.svelte';

/** One part of a Settings page's form: a settings model (a `SettingsForm`: a group's, or a list's). */
export type PagePart = {
	/** What the status line calls it when a later part fails ("Schedule saved."). */
	readonly label: string;
	readonly dirty: boolean;
	/** False while the part's draft is known to be invalid (Save stays disabled). */
	readonly canSave: boolean;
	/**
	 * Why the page cannot be saved as it is, whichever part is unsaved: a feature switched on
	 * that cannot work ("Add at least one recipient first."); `null` when it can. The save bar
	 * shows it beside a disabled Save.
	 */
	readonly blocked?: string | null;
	/** Why the part holds Save (`blocked`, else why it cannot be saved); the save bar states it. */
	readonly held?: { readonly message: string } | null;
	/** Saved after the other parts: a group whose switch, switched on, needs them saved first. */
	readonly saveLast?: boolean;
	/** Save the part's draft; a failure keeps it dirty with its error. */
	persist(): Promise<Feedback>;
	discard(): void;
};

/** "Schedule", "Schedule and Recipients", "A, B and C". */
export function listLabels(labels: string[]): string {
	return labels.length < 2 ? labels.join('') : `${labels.slice(0, -1).join(', ')} and ${labels.at(-1)}`;
}

/**
 * A Settings page's one form: the parts it holds (in page order, as they register), whether any
 * is unsaved, and one save for all of them. Save runs the dirty parts in order (a `saveLast`
 * part after the others) and stops at the first failure: the parts before it are saved and
 * clean (named in the status line), the failed one stays dirty with its error. Discard takes
 * every part back.
 */
export class PageForm {
	/** The page's save; its feedback is the save bar's status line. */
	readonly save = new Action();
	/** Each part as its component reads it (a model handed in as a prop). */
	#parts = $state.raw<(() => PagePart)[]>([]);

	get #list(): PagePart[] {
		return this.#parts.map((part) => part());
	}

	/** Add a part (after the ones already there); returns its removal. */
	add(part: () => PagePart): () => void {
		this.#parts = [...this.#parts, part];
		return () => {
			this.#parts = this.#parts.filter((p) => p !== part);
		};
	}

	/** A part has unsaved edits. */
	get dirty(): boolean {
		return this.#list.some((p) => p.dirty);
	}

	/** Why the unsaved page cannot be saved as it is (the first part's reason); `null` when it can. */
	get blocked(): string | null {
		if (!this.dirty) return null;
		return this.#list.find((p) => p.blocked)?.blocked ?? null;
	}

	/**
	 * Why Save is disabled while the page is unsaved, said in the save bar: the page's `blocked`,
	 * else the first unsaved part that cannot be saved (its `held`); `null` when Save may run.
	 * Every disabled Save has one, so none is disabled silently.
	 */
	get reason(): string | null {
		const blocked = this.blocked;
		if (blocked !== null || !this.dirty) return blocked;
		const stuck = this.#list.find((p) => p.dirty && !p.canSave);
		return stuck ? (stuck.held?.message ?? SAVE_HELD) : null;
	}

	/** Every unsaved part may be saved, and nothing blocks the page. */
	get canSave(): boolean {
		return this.reason === null;
	}

	/** Back to the saved values everywhere; clears the status line. */
	discard(): void {
		for (const part of this.#list) part.discard();
		this.save.feedback = null;
	}

	/** The page form's submit: save the dirty parts in order, stopping at the first failure. */
	submit(event?: SubmitEvent): void {
		event?.preventDefault();
		void this.save.run(async () => {
			const saved: string[] = [];
			const messages: string[] = [];
			const parts = this.#list;
			const order = [...parts.filter((p) => !p.saveLast), ...parts.filter((p) => p.saveLast)];
			for (const part of order) {
				if (!part.dirty) continue;
				const result = await part.persist();
				if (!result.ok)
					return {
						ok: false,
						message: saved.length > 0 ? `${listLabels(saved)} saved. ${part.label}: ${result.message}` : result.message
					};
				saved.push(part.label);
				if (result.message !== 'Saved') messages.push(result.message);
			}
			return { ok: true, message: messages.join(' · ') || 'Saved' };
		});
	}
}

const KEY = Symbol('page-form');

/** Make `form` the page form of the components below (the Settings page creates it). */
export function setPageForm(form: PageForm): void {
	setContext(KEY, form);
}

/**
 * Join the page form around this component with `part` (its settings model, as the component
 * reads it), for as long as the component is mounted; without one (a test, the setup), nothing
 * happens. Call during component initialisation: the parts join in the order they are created,
 * which is page order.
 */
export function joinPageForm(part: () => PagePart): void {
	const form = getContext<PageForm | undefined>(KEY);
	if (!form) return;
	onDestroy(form.add(part));
}
