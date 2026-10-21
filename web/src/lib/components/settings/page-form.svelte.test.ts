// A Settings page's one form: Save runs the unsaved parts in page order and stops at the first
// failure, naming what was saved; Discard takes every part back.
import { describe, expect, it } from 'vitest';
import type { Feedback } from '../action.svelte';
import { listLabels, PageForm, type PagePart } from './page-form.svelte';

/** Let promises settle (a macrotask later). */
const settle = () => new Promise<void>((done) => setTimeout(done));

/** Join `p` to `form`, as a component joins its model. */
const add = (form: PageForm, p: PagePart) => form.add(() => p);

function part(label: string, result: Feedback | null, log: string[], canSave = true): PagePart & { dirty: boolean } {
	return {
		label,
		dirty: result !== null,
		canSave,
		async persist() {
			log.push(label);
			if (result?.ok) this.dirty = false;
			return result ?? { ok: true, message: 'Saved' };
		},
		discard() {
			log.push(`discard ${label}`);
			this.dirty = false;
		}
	};
}

describe('PageForm', () => {
	it('saves the unsaved parts in order and joins what they say', async () => {
		const log: string[] = [];
		const form = new PageForm();
		const a = part('Schedule', { ok: true, message: 'Saved' }, log);
		add(form, a);
		add(form, part('Untouched', null, log));
		add(form, part('Recipients', { ok: true, message: 'Saved: 1 recipient starts' }, log));
		expect(form.dirty).toBe(true);
		form.submit();
		await settle();
		expect(log).toEqual(['Schedule', 'Recipients']);
		expect(form.save.feedback).toEqual({ ok: true, message: 'Saved: 1 recipient starts' });
	});

	it('stops at the first failure and names the parts saved before it', async () => {
		const log: string[] = [];
		const form = new PageForm();
		add(form, part('Schedule', { ok: true, message: 'Saved' }, log));
		add(form, part('Recipients', { ok: false, message: 'Line 2: no such organization' }, log));
		add(form, part('Later', { ok: true, message: 'Saved' }, log));
		form.submit();
		await settle();
		expect(log).toEqual(['Schedule', 'Recipients']);
		expect(form.save.feedback).toEqual({
			ok: false,
			message: 'Schedule saved. Recipients: Line 2: no such organization'
		});
	});

	it('says a lone failure as it is, and Save waits for every unsaved part to be valid', async () => {
		const log: string[] = [];
		const form = new PageForm();
		add(form, part('Mailbox', { ok: false, message: 'Login refused.' }, log));
		form.submit();
		await settle();
		expect(form.save.feedback).toEqual({ ok: false, message: 'Login refused.' });
		const remove = add(form, part('Bad', { ok: true, message: 'Saved' }, log, false));
		expect(form.canSave).toBe(false);
		// Never silent: a part that names no reason still gets one.
		expect(form.reason).toBe('Correct the marked fields first.');
		remove();
		expect(form.canSave).toBe(true);
		add(form, { ...part('Days', { ok: true, message: 'Saved' }, log, false), held: { message: 'Keep 1 to 120.' } });
		expect(form.reason).toBe('Keep 1 to 120.');
	});

	it('saves a saveLast part after the others, whatever its place', async () => {
		const log: string[] = [];
		const form = new PageForm();
		add(form, { ...part('Schedule', { ok: true, message: 'Saved' }, log), saveLast: true });
		add(form, part('Recipients', { ok: true, message: 'Saved' }, log));
		form.submit();
		await settle();
		expect(log).toEqual(['Recipients', 'Schedule']);
	});

	it("reads a part's saveLast at each save (client reports: last when switched on)", async () => {
		const log: string[] = [];
		const on = { value: true };
		const schedule = part('Schedule', { ok: false, message: 'Refused.' }, log);
		const form = new PageForm();
		add(
			form,
			Object.defineProperty(schedule, 'saveLast', {
				get: () => on.value
			})
		);
		add(form, part('Recipients', { ok: false, message: 'Refused.' }, log));
		form.submit();
		await settle();
		expect(log).toEqual(['Recipients']);
		on.value = false;
		form.submit();
		await settle();
		expect(log).toEqual(['Recipients', 'Schedule']);
	});

	it('a blocked page waits with its reason while unsaved, whichever part is unsaved', () => {
		const log: string[] = [];
		const form = new PageForm();
		const reason = { blocked: 'Add at least one recipient first.' as string | null };
		const schedule = part('Schedule', null, log);
		add(form, {
			...schedule,
			get blocked() {
				return reason.blocked;
			}
		});
		expect(form.blocked).toBeNull();
		const recipients = part('Recipients', { ok: true, message: 'Saved' }, log);
		add(form, recipients);
		expect(form.blocked).toBe('Add at least one recipient first.');
		expect(form.reason).toBe('Add at least one recipient first.');
		expect(form.canSave).toBe(false);
		reason.blocked = null;
		expect(form.blocked).toBeNull();
		expect(form.canSave).toBe(true);
	});

	it('discards every part', () => {
		const log: string[] = [];
		const form = new PageForm();
		add(form, part('A', { ok: true, message: 'Saved' }, log));
		add(form, part('B', null, log));
		form.discard();
		expect(log).toEqual(['discard A', 'discard B']);
		expect(form.dirty).toBe(false);
	});
});

describe('listLabels', () => {
	it('lists names in words', () => {
		expect(listLabels(['A'])).toBe('A');
		expect(listLabels(['A', 'B'])).toBe('A and B');
		expect(listLabels(['A', 'B', 'C'])).toBe('A, B and C');
	});
});
