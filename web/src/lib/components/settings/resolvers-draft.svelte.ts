import { saveDns } from '$lib/api';
import { settings } from '$lib/state/settings.svelte';
import type { DnsSettingsResponse } from '$lib/types/DnsSettingsResponse';
import type { SettingsResponse } from '$lib/types/SettingsResponse';
import { SettingsForm, type SettingsFormSource, type Writes } from './settings-form.svelte';

/** The DNS checks page's two resolver fields. */
export type ResolversDraft = { primary: string; secondary: string };

/** The resolvers in use: the stored ones, else the defaults. */
export function inUse(dns: DnsSettingsResponse): string[] {
	return dns.nameservers.length > 0 ? dns.nameservers : dns.defaults;
}

/** The resolvers typed, trimmed, without empty fields: what is saved (none: the defaults). */
export function typed(d: ResolversDraft): string[] {
	return [d.primary.trim(), d.secondary.trim()].filter(Boolean);
}

/** The DNS checks page's resolvers (the dns group), saved through `settings.save`. */
export class ResolversModel extends SettingsForm<SettingsResponse, ResolversDraft> {
	static readonly writes: Writes = { page: 'dns-checks', groups: ['dns'] };

	constructor(options: { source: SettingsFormSource<SettingsResponse> }) {
		super({ ...options.source, label: 'Resolvers', blank: { primary: '', secondary: '' } });
	}

	protected toDraft(s: SettingsResponse): ResolversDraft {
		const [primary = '', secondary = ''] = inUse(s.dns);
		return { primary, secondary };
	}

	/** A lone secondary moves up. */
	protected normalize(d: ResolversDraft): ResolversDraft {
		const [primary = '', secondary = ''] = typed(d);
		return { primary, secondary };
	}

	protected async write(d: ResolversDraft): Promise<{ saved: SettingsResponse }> {
		return { saved: await settings.save(saveDns(typed(d))) };
	}
}
