import { apiErrorMessage } from '$lib/http';

export type Feedback = { ok: boolean; message: string };

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
			this.feedback = { ok: false, message: apiErrorMessage(err) };
		} finally {
			this.busy = false;
		}
	}
}
