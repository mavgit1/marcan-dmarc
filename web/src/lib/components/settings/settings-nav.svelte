<script lang="ts">
	import { resolve } from '$app/paths';
	import { getDkimSelectors, getSenderNames } from '$lib/api';
	import { plural } from '$lib/format';
	import { SETTINGS_GROUPS, SETTINGS_PAGES, type SettingsPageId } from '$lib/settings-nav';
	import { resource } from '$lib/state/resource.svelte';
	import { scopeOptions } from '$lib/state/scope.svelte';
	import { settings } from '$lib/state/settings.svelte';
	import { systemHealth } from '$lib/state/system-health.svelte';
	import { cn } from '$lib/utils.js';
	import ChevronRightIcon from '@lucide/svelte/icons/chevron-right';
	import { countEntries } from './text-lines';

	/**
	 * The Settings pages by group, each with one line of state in muted text: the nav column
	 * beside a page from `lg` up (`column`), and the whole of `/settings` below it (`list`).
	 */
	let { variant, active }: { variant: 'column' | 'list'; active?: SettingsPageId } = $props();

	$effect(() => {
		settings.load().catch(() => {});
		scopeOptions.load().catch(() => {});
	});
	const senderNames = resource({ key: () => 'settings-nav-sender-names', load: getSenderNames });
	const selectors = resource({ key: () => 'settings-nav-dkim-selectors', load: getDkimSelectors });

	const onOff = (on: boolean | undefined) => (on === undefined ? null : { text: on ? 'On' : 'Off' });

	/** Each page's state: whether a feature is on, how many entries a list has. */
	const lines = $derived.by((): Partial<Record<SettingsPageId, { text: string; fail?: boolean } | null>> => {
		const s = settings.current;
		const failed = systemHealth.current?.pipeline.mailbox === 'error';
		return {
			mailbox: s && {
				text: !s.mailbox.configured ? 'Not set up' : failed ? 'Check failed' : 'Connected',
				fail: s.mailbox.configured && failed
			},
			sending: onOff(s?.sending.enabled),
			alerts: onOff(s?.alerts.enabled),
			'client-reports': onOff(s?.client_reports.enabled),
			domains: scopeOptions.current && { text: plural(scopeOptions.current.domains.length, 'domain') },
			'sender-names': senderNames.data && { text: plural(countEntries(senderNames.data.text), 'name') },
			'dns-checks': selectors.data && { text: plural(countEntries(selectors.data.text), 'extra selector') }
		};
	});
</script>

<nav aria-label="Settings" class={variant === 'column' ? 'space-y-5' : 'space-y-6'}>
	{#each SETTINGS_GROUPS as group (group.id)}
		<div>
			<h2 class={cn('text-muted-foreground text-xs', variant === 'column' ? 'mb-1 px-2' : 'mb-2 font-medium')}>
				{group.label}
			</h2>
			<ul class={variant === 'list' ? 'bg-card ring-foreground/10 divide-y rounded-xl ring-1' : 'space-y-0.5'}>
				{#each SETTINGS_PAGES.filter((p) => p.group === group.id) as page (page.id)}
					{@const line = lines[page.id]}
					<li>
						<a
							href={resolve(`/settings/${page.id}`)}
							aria-current={active === page.id ? 'page' : undefined}
							class={cn(
								'text-foreground focus-visible:ring-ring flex min-w-0 cursor-pointer gap-3 text-sm outline-none transition-colors focus-visible:ring-2',
								variant === 'column'
									? 'hover:bg-sidebar-accent h-8 items-center rounded-md px-2'
									: 'hover:bg-muted/40 items-baseline px-4 py-3 focus-visible:ring-inset',
								variant === 'column' && active === page.id && 'bg-sidebar-accent font-medium',
								variant === 'list' && active === page.id && 'bg-muted font-medium'
							)}
						>
							<span class="min-w-0 flex-1 truncate">{page.title}</span>
							{#if line}
								<span
									class={cn(
										'shrink-0 text-xs tabular-nums',
										line.fail ? 'text-status-fail-fg' : 'text-muted-foreground'
									)}
								>
									{line.text}
								</span>
							{/if}
							{#if variant === 'list'}
								<ChevronRightIcon class="text-muted-foreground size-4 shrink-0 self-center" />
							{/if}
						</a>
					</li>
				{/each}
			</ul>
		</div>
	{/each}
</nav>
