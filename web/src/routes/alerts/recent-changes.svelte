<script lang="ts">
	import { resolve } from '$app/paths';
	import { getAlerts } from '$lib/api';
	import { relativeTime } from '$lib/format';
	import { scopedHref } from '$lib/state/filters.svelte';
	import { resource } from '$lib/state/resource.svelte';
	import AlertText from './alert-text.svelte';

	/**
	 * A domain's last few changes (the domain overview), whatever the time range, with a link to
	 * the alert list narrowed to the domain: the link names the organization and the domain, so it
	 * opens the same in a new tab. Renders nothing while there are none.
	 */
	let { domainId, organizationId }: { domainId: number; organizationId: number } = $props();

	const RECENT = 5;

	const recent = resource({
		key: () => String(domainId),
		load: () => getAlerts({ domain_id: domainId, limit: RECENT })
	});

	const allHref = $derived(resolve(scopedHref('/alerts', { organizationId, domainId }) as '/alerts'));
</script>

{#if recent.data && recent.data.total > 0}
	<section class="rounded-lg border p-4">
		<div class="mb-2 flex items-baseline justify-between gap-3">
			<h2 class="text-sm font-medium">Recent changes</h2>
			<a href={allHref} class="text-muted-foreground text-xs hover:underline"> All changes of this domain </a>
		</div>
		<ul class="space-y-1.5 text-sm">
			{#each recent.data.items as item (item.id)}
				<li class="flex items-baseline gap-3">
					<span class="text-muted-foreground w-16 shrink-0 text-xs tabular-nums">{relativeTime(item.created_at)}</span>
					<AlertText {item} />
				</li>
			{/each}
		</ul>
	</section>
{/if}
