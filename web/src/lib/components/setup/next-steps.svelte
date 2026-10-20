<script lang="ts">
	import SettingsLink from '$lib/components/settings/settings-link.svelte';

	/**
	 * "Optional next steps" after the setup: alerts, monthly client reports, and the operator key
	 * for whoever runs Marcan for clients. The setup's Done step shows it, in /setup and its guide.
	 * `heading` is the heading element's level in its page.
	 */
	let { heading = 'h3' }: { heading?: 'h2' | 'h3' } = $props();
</script>

<div class="space-y-2">
	<svelte:element this={heading} class={heading === 'h2' ? 'text-sm font-medium' : 'text-xs font-semibold'}>
		Optional next steps
	</svelte:element>
	<ul class="text-muted-foreground list-disc space-y-1 pl-5 text-xs leading-relaxed">
		<li>
			<span class="text-foreground">Turn on alerts</span>: an email when something about a domain changes, and a digest.
			Set up <SettingsLink to="sending">Settings → Sending</SettingsLink>, then
			<SettingsLink to="alerts">Alerts</SettingsLink>.
		</li>
		<li>
			<span class="text-foreground">Send clients a monthly report</span>: list who gets which organization's report
			under <SettingsLink to="client-reports">Settings → Client reports</SettingsLink>, then turn it on there.
		</li>
		<li>
			<span class="text-foreground">Running Marcan for clients?</span> Add your operator key under
			<SettingsLink to="operator-key">Settings → License</SettingsLink>, or keep the free-software notice on.
		</li>
	</ul>
</div>
