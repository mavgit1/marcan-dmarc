<script lang="ts">
	import { statusText } from '$lib/tokens';
	import SettingsLink from './settings-link.svelte';

	/**
	 * The line under a test or preview button: where it goes, the test recipient set under
	 * Sending ("Goes to me@example.com. Change it under Sending."). On the Sending page itself
	 * (`here`), without the link: the field is on the page.
	 */
	let { to, here = false }: { to: string; here?: boolean } = $props();
</script>

{#if to}
	<p class={['mt-1 text-[11px]', statusText.neutral]}>
		Goes to {to}.
		{#if !here}
			Change it under <SettingsLink to="sending" class="text-inherit">Sending</SettingsLink>.
		{/if}
	</p>
{/if}
