import { saveUpdates } from '$lib/api';
import { meta } from '$lib/state/meta.svelte';
import { settings } from '$lib/state/settings.svelte';
import type { SettingsResponse } from '$lib/types/SettingsResponse';
import type { UpdateSettings } from '$lib/types/UpdateSettings';
import { SettingsForm, type SettingsFormSource, type Writes } from './settings-form.svelte';

/**
 * The License page's version check (the updates group): one switch, on by default, saved by the
 * page's save bar through `settings.save`. The server fetches, compares and says whether a newer
 * version is out (`meta`'s `available`, which the sidebar shows too); every save reloads `meta` so
 * both follow the switch. The browser never fetches anything for it.
 */
export class UpdatesModel extends SettingsForm<SettingsResponse, UpdateSettings> {
	static readonly writes: Writes = { page: 'license', groups: ['updates'] };

	constructor(options: { source: SettingsFormSource<SettingsResponse> }) {
		super({ ...options.source, label: 'Version check', blank: { check: true } });
	}

	protected toDraft(s: SettingsResponse): UpdateSettings {
		return { check: s.updates.check };
	}

	protected async write(d: UpdateSettings): Promise<{ saved: SettingsResponse }> {
		const saved = await settings.save(saveUpdates(d));
		// Saved either way; a failed reload only leaves the notice as it was until the next load.
		meta.reload().catch(() => {});
		return { saved };
	}
}
