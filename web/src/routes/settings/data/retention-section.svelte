<script lang="ts">
	import { joinPageForm } from '$lib/components/settings/page-form.svelte';
	import type { RetentionModel } from '$lib/components/settings/retention-draft.svelte';
	import SettingRow from '$lib/components/settings/setting-row.svelte';
	import SettingsPart from '$lib/components/settings/settings-part.svelte';
	import SettingsSection from '$lib/components/settings/settings-section.svelte';
	import { Input } from '$lib/components/ui/input/index.js';
	import { pluralNoun } from '$lib/format';
	import { meta } from '$lib/state/meta.svelte';
	import { statusText } from '$lib/tokens';

	/** The Data page's Retention section (`model`, the retention group), saved by the page's save bar. */
	let { model: m }: { model: RetentionModel } = $props();
	$effect(() => {
		meta.load().catch(() => {});
	});
	joinPageForm(() => m);
</script>

<SettingsSection title="Retention" anchor="retention">
	<SettingRow title="Reports" hint="Older ones are deleted once a day.">
		<SettingsPart form={m} errorTitle="Retention failed to load" skeleton="h-8 w-60">
			<label class="flex flex-wrap items-center gap-2 text-sm">
				Keep reports for
				<Input
					type="number"
					min={m.range?.min}
					max={m.range?.max}
					step="1"
					bind:value={m.draft.report_retention_months}
					aria-invalid={m.ariaInvalid('report_retention_months')}
					aria-label="Months to keep reports"
					class="h-8 w-20 text-sm"
				/>
				{pluralNoun(Number(m.draft.report_retention_months), 'month')}
			</label>
			{#if m.problem}
				<p class={['mt-1 text-[11px]', statusText.warn]}>{m.problem}</p>
			{/if}
		</SettingsPart>
	</SettingRow>
</SettingsSection>
