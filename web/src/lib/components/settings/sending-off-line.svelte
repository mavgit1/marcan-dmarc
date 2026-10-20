<script lang="ts">
	import { settings } from '$lib/state/settings.svelte';
	import { statusText } from '$lib/tokens';
	import SettingsLink from './settings-link.svelte';

	/**
	 * The one dependency line under the Alerts and Client reports switches, while sending is off
	 * as saved: nothing can be mailed until it is on.
	 */
	const off = $derived(settings.current !== null && !settings.current.sending.enabled);
</script>

{#if off}
	<p class={['text-xs', statusText.warn]}>
		<SettingsLink to="sending" class="text-inherit">Sending</SettingsLink> is off, so nothing is mailed.
	</p>
{/if}
