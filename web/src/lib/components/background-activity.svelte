<script lang="ts">
	import { formatInt, plural } from '$lib/format';
	import { importActivityLabel } from '$lib/imap-progress';
	import { systemHealth } from '$lib/state/system-health.svelte';
	import { cn } from '$lib/utils.js';
	import Loader2Icon from '@lucide/svelte/icons/loader-2';

	/**
	 * Compact lines while background work changes what the page shows: a mailbox import ("Importing
	 * the mailbox: 1,240 reports so far") and the first DNS checks of new domains ("Checking DNS: 12
	 * of 40 domains checked"). From the shared health poll; renders nothing while idle.
	 */
	let {
		imports = 'bulk',
		class: className
	}: {
		/** Which running mailbox passes show: any, a bulk import only (the server's `bulk`), or none. */
		imports?: 'any' | 'bulk' | 'none';
		class?: string;
	} = $props();

	const health = $derived(systemHealth.current);
	const pass = $derived.by(() => {
		const p = health?.imap_progress;
		if (!p || imports === 'none') return null;
		return imports === 'any' || p.bulk ? p : null;
	});
	const dns = $derived(health?.dns_checks.checking ? health.dns_checks : null);
</script>

{#if pass || dns}
	<div class={cn('text-muted-foreground space-y-0.5 text-xs', className)} role="status" aria-live="polite">
		{#if pass}
			<p class="flex items-center gap-1.5">
				<Loader2Icon class="size-3.5 shrink-0 animate-spin" aria-hidden="true" />
				<span class="tabular-nums">{importActivityLabel(pass)}</span>
			</p>
		{/if}
		{#if dns}
			<p class="flex items-center gap-1.5">
				<Loader2Icon class="size-3.5 shrink-0 animate-spin" aria-hidden="true" />
				<span class="tabular-nums">
					Checking DNS: {formatInt(dns.checked)} of {plural(dns.domains, 'domain')} checked
				</span>
			</p>
		{/if}
	</div>
{/if}
