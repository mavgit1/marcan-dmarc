import { apiErrorMessage, errorBody } from '#lib/http.ts';

/** `docs`: a failure's docs section on the fix, as the server's error answer names it (`ErrorBody.docs`). */
export type Feedback = { ok: boolean; message: string; docs?: string | null };

/** Busy flag and pass/fail feedback for one async settings action. */
export class Action {
	busy = $state(false);
	feedback = $state.raw<Feedback | null>(null);

	/** Runs `fn`. A returned value becomes the feedback; a thrown error becomes a failure. */
	async run(fn: () => Promise<Feedback | void>) {
		this.busy = true;
		this.feedback = null;
		try {
			const result = await fn();
			if (result) this.feedback = result;
		} catch (err) {
			this.feedback = { ok: false, message: apiErrorMessage(err), docs: errorBody(err)?.docs ?? null };
		} finally {
			this.busy = false;
		}
	}
}
