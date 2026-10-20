import { saveLicense } from '$lib/api';
import { meta } from '$lib/state/meta.svelte';
import { settings } from '$lib/state/settings.svelte';
import type { StatusTone } from '$lib/tokens';
import type { LicenseState } from '$lib/types/LicenseState';
import type { SaveLicenseRequest } from '$lib/types/SaveLicenseRequest';
import type { SettingsResponse } from '$lib/types/SettingsResponse';
import { SettingsForm, type SettingsFormSource, type Writes } from './settings-form.svelte';

/** The editable fields of the license group. The stored key is never sent back, so `key` starts empty. */
export type LicenseDraft = {
	/** A new key to store; empty: keep the stored one. */
	key: string;
	/** Remove the stored key on save. */
	remove: boolean;
};

export const BLANK: LicenseDraft = { key: '', remove: false };

/** The request for a draft: a typed key replaces the stored one, else `remove` clears it, else it is kept. */
export function toRequest(d: LicenseDraft): SaveLicenseRequest {
	const key = d.key.trim();
	if (key) return { operator_key: key };
	if (d.remove) return { operator_key: '' };
	return {};
}

/** The License page's license state box for one server state. */
export type LicenseStateView = {
	tone: StatusTone;
	/** The first sentence, in bold. */
	title: string;
	/** The sentence after "Licensed to <email>." when the key names someone; `null`: none. */
	detail: string | null;
	/** Managing the subscription and Branding (while active), or where a license is bought. */
	links: 'manage' | 'buy';
};

/** The License page's license state box per server state (`license.state`). */
export const LICENSE_STATE: Record<LicenseState, LicenseStateView> = {
	active: {
		tone: 'pass',
		title: 'Operator license active.',
		detail: 'Client emails carry your name, not the free-software notice.',
		links: 'manage'
	},
	no_key: {
		tone: 'neutral',
		title: 'No operator license.',
		detail: 'Client emails carry the free-software notice.',
		links: 'buy'
	},
	invalid: { tone: 'fail', title: 'The stored operator key is not valid.', detail: null, links: 'buy' }
};

/**
 * The License page's model (the license group): the key field and removing the stored key,
 * saved through `settings.save`; every save reloads `meta` so the app frame follows (a valid key
 * switches the free-software notice off, removing it brings the notice back).
 */
export class LicenseModel extends SettingsForm<SettingsResponse, LicenseDraft> {
	static readonly writes: Writes = { page: 'license', groups: ['license'] };
	/** Who the stored key names; `null` without a valid key. */
	licensedTo: string | null = $derived.by(() => this.saved?.license.licensed_to ?? null);
	/** A key is stored (valid or not). */
	hasKey: boolean = $derived.by(() => !!this.saved?.license.has_key);
	/** The stored key as the server names it ("you@acme.example · …GPL7CQ"); `null` without one. */
	keyHint: string | null = $derived.by(() => this.saved?.license.key_hint ?? null);

	constructor(options: { source: SettingsFormSource<SettingsResponse> }) {
		super({ ...options.source, label: 'License', blank: BLANK });
	}

	/** Nothing typed, nothing removed, whatever is stored (the key is never sent back). */
	protected toDraft(): LicenseDraft {
		return { ...BLANK };
	}

	protected normalize(d: LicenseDraft): LicenseDraft {
		return { ...d, key: d.key.trim() };
	}

	/** Mark the stored key for removal (the notice comes back on save). */
	removeKey(): void {
		this.draft.remove = true;
		this.draft.key = '';
	}

	protected async write(d: LicenseDraft): Promise<{ saved: SettingsResponse; message?: string }> {
		const request = toRequest(d);
		const saved = await settings.save(saveLicense(request));
		meta.reload().catch(() => {});
		const message =
			request.operator_key === ''
				? 'Key removed'
				: request.operator_key && saved.license.licensed_to
					? `Saved: licensed to ${saved.license.licensed_to}`
					: undefined;
		return { saved, message };
	}
}
