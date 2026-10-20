<script lang="ts">
	import SettingRow from '$lib/components/settings/setting-row.svelte';
	import SettingsSection from '$lib/components/settings/settings-section.svelte';
	import { formatInt, formatMegabytes } from '$lib/format';
	import { systemHealth } from '$lib/state/system-health.svelte';

	/** The Data page's Stored data section: what the database holds, read-only. */
	const health = $derived(systemHealth.current);
</script>

<SettingsSection title="Stored data" anchor="stored-data">
	<SettingRow title="Database">
		{#if health}
			<span class="tabular-nums">
				{formatInt(health.total_reports)} reports · {formatInt(health.total_messages)} messages · {formatMegabytes(
					health.database_size_bytes
				)}
			</span>
		{:else}
			—
		{/if}
	</SettingRow>
</SettingsSection>
