<script lang="ts">
	import type { LookupStatus } from '$lib/types/LookupStatus';

	/** Raw outcome of one DNS or HTTP fetch as plain text, e.g. `DNS: NXDOMAIN`. Not a verdict. */
	let { lookup, prefix = 'DNS' }: { lookup: LookupStatus; prefix?: string } = $props();
</script>

<span class="text-muted-foreground text-[11px]">
	{prefix}: <span class="font-mono">{lookup.toUpperCase()}</span>
</span>
