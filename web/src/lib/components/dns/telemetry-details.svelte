<script lang="ts">
	import { resolve } from '$app/paths';
	import type { WithIdentity } from '$lib/types/WithIdentity';
	import type { DomainFailureRow } from '$lib/types/DomainFailureRow';
	import AuthChecks from '$lib/components/auth-checks.svelte';
	import Skeleton from '$lib/components/skeleton.svelte';
	import { Button } from '$lib/components/ui/button/index.js';
	import { formatInt } from '$lib/format';
	import ArrowRightIcon from '@lucide/svelte/icons/arrow-right';
	import DetailHeading from './detail-heading.svelte';

	type DomainFailureItem = WithIdentity<DomainFailureRow>;

	/**
	 * The Report data drawer below its findings: where the delivered failures come from. The
	 * figures themselves are on the card; this only adds the sources, in the failures tab's words.
	 */
	let {
		failures,
		failuresHref
	}: {
		/** Top delivered failures (forwarded mail excluded); `null` while loading. */
		failures: DomainFailureItem[] | null;
		/** The Failures tab filtered to the same records. */
		failuresHref: string;
	} = $props();
</script>

<div class="space-y-2">
	<div class="flex items-center justify-between gap-2">
		<DetailHeading>Top delivered failures</DetailHeading>
		<Button variant="outline" size="sm" class="h-7 shrink-0 gap-1.5 text-xs" href={failuresHref}>
			All in Failures
			<ArrowRightIcon class="size-3" />
		</Button>
	</div>

	{#if failures && failures.length > 0}
		<div class="overflow-x-auto rounded-lg border border-border">
			<table class="w-full text-left text-xs">
				<thead class="bg-muted/40 text-muted-foreground border-b border-border font-medium">
					<tr>
						<th class="p-2.5">Source IP</th>
						<th class="p-2.5">SPF</th>
						<th class="p-2.5">DKIM</th>
						<th class="p-2.5 text-right">Messages</th>
					</tr>
				</thead>
				<tbody class="divide-y divide-border">
					{#each failures as item, i (i)}
						<tr class="hover:bg-muted/10">
							<td class="p-2.5">
								<a
									href={resolve(`/senders/${encodeURIComponent(item.source_ip)}`)}
									class="text-foreground font-mono font-medium hover:underline"
								>
									{item.source_ip}
								</a>
							</td>
							<td class="p-2.5"><AuthChecks kind="spf" checks={item.spf_auth} /></td>
							<td class="p-2.5"><AuthChecks kind="dkim" checks={item.dkim_auth} /></td>
							<td class="text-foreground p-2.5 text-right tabular-nums">{formatInt(item.count)}</td>
						</tr>
					{/each}
				</tbody>
			</table>
		</div>
	{:else if !failures}
		<Skeleton variant="block" class="h-28" />
	{:else}
		<p class="text-muted-foreground text-xs">No delivered failures in the selected window.</p>
	{/if}
</div>
