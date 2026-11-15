import { redirect } from '@sveltejs/kit';
import { resolve } from '$app/paths';

/** The 0.3 Mailbox page is the Accounts page now; old links land there. */
export function load() {
	redirect(308, resolve('/settings/accounts'));
}
