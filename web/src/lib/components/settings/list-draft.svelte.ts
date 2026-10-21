import { apiLineErrors, textSaveError } from '$lib/http';
import type { LineError } from '$lib/types/LineError';
import type { ListText } from '$lib/types/ListText';
import { plural } from '$lib/format';
import { SettingsForm, type Invalid } from './settings-form.svelte';
import { countEntries } from './text-lines';

/** How long typing must pause before the draft is previewed. */
export const LIST_PREVIEW_DELAY_MS = 350;

/** What saving a list would change, worded for the editor's "Saving will change" box. */
export type ListChanges = {
	/** "4 moved, 1 added"; empty when saving changes nothing. */
	summary: string;
	/**
	 * The details: a group with a `label` is folded under "Moved (4)", one without is a plain
	 * list of lines.
	 */
	details: { label?: string; items: string[] }[];
};

/** The server's summary of a list: what saving changes and the lines it cannot read. */
export type ListSummary = { errors: LineError[] };

/**
 * One list edited as text (`ListText` on the server): its three requests and how it is worded.
 * The four lists (`settings/lists.ts`) differ only in this.
 */
export type ListSpec<S extends ListSummary> = {
	/** The page part's name in the save bar's status line ("Recipients"). */
	label: string;
	/** The editor's accessible name and its load error ("Client report recipients"); default `label`. */
	name?: string;
	/** Distinct per list (the resource key). */
	key: string;
	get: () => Promise<ListText>;
	preview: (text: string) => Promise<S>;
	save: (text: string) => Promise<S>;
	/** Two texts hold the same list (no unsaved change); default: the same text. */
	same?: (a: string, b: string) => boolean;
	/** What saving would change, worded; without it only the line errors are shown. */
	changes?: (summary: S) => ListChanges;
	placeholder?: string;
};

/**
 * A list edited as text (a `SettingsForm` over the stored text): the preview of what saving the
 * draft would change (asked from the server's `…/preview` once typing pauses; a later edit
 * discards an earlier answer), the line errors to show (a refused save's, else the preview's,
 * which hold Save), and the save (`PUT`, then the list read back as stored). Create it during
 * component initialisation (it owns effects); `ListEditor` shows it and joins it to the page form.
 */
export class ListDraft<S extends ListSummary> extends SettingsForm<string, { text: string }> {
	readonly spec: ListSpec<S>;
	#summary = $state.raw<S | null>(null);
	#previewed = $state<string | null>(null);
	#saveErrors = $state.raw<LineError[]>([]);
	#seq = 0;

	constructor(spec: ListSpec<S>) {
		super({
			label: spec.label,
			key: spec.key,
			load: () => spec.get().then((list) => list.text),
			blank: { text: '' },
			saveError: textSaveError
		});
		this.spec = spec;
		$effect(() => {
			const text = this.draft.text;
			const id = ++this.#seq;
			this.#saveErrors = [];
			if (!this.dirty) {
				this.#summary = null;
				return;
			}
			const timer = setTimeout(async () => {
				try {
					const summary = await spec.preview(text);
					if (id !== this.#seq) return;
					this.#summary = summary;
					this.#previewed = text;
				} catch {
					// Saving reports the error; the preview only informs.
				}
			}, LIST_PREVIEW_DELAY_MS);
			return () => clearTimeout(timer);
		});
	}

	protected toDraft(text: string) {
		return { text };
	}

	protected isDirty(draft: { text: string }, base: { text: string }): boolean {
		return this.spec.same ? !this.spec.same(draft.text, base.text) : draft.text !== base.text;
	}

	protected async write({ text }: { text: string }): Promise<{ saved: string; message?: string }> {
		let applied: S;
		try {
			applied = await this.spec.save(text);
		} catch (err) {
			this.#saveErrors = apiLineErrors(err);
			throw err;
		}
		const summary = this.spec.changes?.(applied).summary;
		const message = summary === undefined ? 'Saved' : `Saved: ${summary || 'No changes'}`;
		// The server has saved it; a failed read-back only means the list is not shown as stored
		// until the next load.
		try {
			return { saved: (await this.spec.get()).text, message };
		} catch {
			return { saved: text, message: `${message} (reload to see the list as stored)` };
		}
	}

	/** Save waits while there are line errors (the editor lists them under the text, `text`). */
	protected check(): Invalid | null {
		const lines = new Set(this.errors.map((e) => e.line)).size;
		return lines > 0 ? { field: 'text', message: `Fix ${plural(lines, 'line')} before saving.` } : null;
	}

	/** The server's preview of the draft on screen; `null` until it is answered, and for a saved draft. */
	get summary(): S | null {
		return this.#summary !== null && this.#previewed === this.draft.text ? this.#summary : null;
	}

	/** The line errors to show: a refused save's, else the current preview's. */
	get errors(): LineError[] {
		return this.#saveErrors.length > 0 ? this.#saveErrors : (this.summary?.errors ?? []);
	}

	/** What saving the draft would change, worded; `null` while unknown, with line errors or without `changes`. */
	get changes(): ListChanges | null {
		const summary = this.summary;
		if (!summary || summary.errors.length > 0 || !this.spec.changes) return null;
		return this.spec.changes(summary);
	}

	/** Entries in the draft as it would be saved (blank lines and comments left out); 0 before it loads. */
	get count(): number {
		return this.saved === null ? 0 : countEntries(this.draft.text);
	}
}
