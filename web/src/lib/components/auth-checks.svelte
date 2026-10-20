<script lang="ts">
	import { checkDisplay, noCheckDisplay, type AuthCheck } from '$lib/auth-display';
	import { statusTextStrong } from '$lib/tokens';
	import { cn } from '$lib/utils.js';

	/**
	 * SPF or DKIM checks of one record in plain words: "Pass", "Pass for <domain>", "Fail"
	 * or "None". The most relevant check leads (the server sends them in that order); any
	 * others follow as a muted "+N" whose title lists them. Labels from `$lib/auth-display`.
	 */
	let { checks, kind, class: className }: { checks: AuthCheck[]; kind: 'spf' | 'dkim'; class?: string } = $props();

	const ranked = $derived(checks);
	const lead = $derived(ranked.length ? checkDisplay(ranked[0], kind) : noCheckDisplay(kind));
	const rest = $derived(ranked.slice(1).map((c) => checkDisplay(c, kind)));
</script>

<span class={cn('inline-flex min-w-0 items-baseline gap-1.5 text-xs whitespace-nowrap', className)}>
	<span class={cn('max-w-[200px] truncate', statusTextStrong[lead.tone])} title={lead.title}>
		{#if ranked[0]?.domain && lead.label === `Pass for ${ranked[0].domain}`}
			Pass for <span class="font-mono">{ranked[0].domain}</span>
		{:else}
			{lead.label}
		{/if}
	</span>
	{#if rest.length}
		<span class="text-muted-foreground tabular-nums" title={rest.map((r) => `${r.label}: ${r.title}`).join('\n')}>
			+{rest.length}
		</span>
	{/if}
</span>
