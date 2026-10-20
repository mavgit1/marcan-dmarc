<script lang="ts">
	import { Button, type ButtonProps } from '$lib/components/ui/button/index.js';
	import Loader2Icon from '@lucide/svelte/icons/loader-2';

	/** Button that swaps its label for a spinner and `busyLabel` while `busy`. */
	let { busy, busyLabel, disabled, children, ...rest }: ButtonProps & { busy: boolean; busyLabel: string } = $props();
</script>

<Button {...rest} disabled={busy || disabled}>
	{#if busy}
		<Loader2Icon class="mr-1.5 size-3.5 animate-spin" />
		{busyLabel}
	{:else}
		{@render children?.()}
	{/if}
</Button>
