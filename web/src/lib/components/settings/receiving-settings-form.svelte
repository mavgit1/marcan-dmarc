<script lang="ts">
	import { Input } from '#lib/components/ui/input/index.ts';
	import { plural, pluralNoun } from '#lib/format.ts';
	import { statusText } from '#lib/tokens.ts';
	import { joinPageForm } from './page-form.svelte';
	import { STOPPED_DAYS, WARN_HOURS, type ReceivingModel } from './receiving-draft.svelte';
	import SettingRow from './setting-row.svelte';
	import { SELECT_CLASS } from './select-class';
	import SettingsPart from './settings-part.svelte';
	import SettingsSection from './settings-section.svelte';

	/**
	 * The Accounts page's form below the cards: Schedule (how often every receiving account is
	 * read) and Report arrival (the report and domain warnings), saved by the page's save bar.
	 * Where reports are sent is each account's own address.
	 */
	let { model: m }: { model: ReceivingModel } = $props();

	joinPageForm(() => m);
	const draft = $derived(m.normalized);
</script>

<SettingsPart form={m} errorTitle="Receiving settings failed to load" skeleton="h-64">
	<SettingsSection title="Schedule">
		<SettingRow title="Checks">
			<label class="flex flex-wrap items-center gap-2 text-sm">
				Check the accounts every
				<Input
					type="number"
					min="1"
					bind:value={m.draft.pollMinutes}
					aria-label="Minutes between checks"
					aria-invalid={m.ariaInvalid('pollMinutes')}
					class="h-8 w-20 text-sm"
				/>
				{pluralNoun(draft.pollMinutes, 'minute')}
			</label>
			{#if m.fieldProblem('pollMinutes')}
				<p class={['mt-1 text-[11px]', statusText.warn]}>{m.fieldProblem('pollMinutes')}</p>
			{/if}
		</SettingRow>
	</SettingsSection>

	<SettingsSection title="Report arrival">
		<SettingRow title="Report warning">
			<label class="flex flex-wrap items-center gap-2 text-sm">
				Warn when no reports at all have arrived for
				<select
					bind:value={m.draft.warnHours}
					aria-label="Hours without any report before the header warns"
					class={[SELECT_CLASS, 'h-8']}
				>
					{#each WARN_HOURS as h (h)}
						<option value={h}>{h} hours</option>
					{/each}
					{#if !WARN_HOURS.includes(draft.warnHours)}
						<option value={draft.warnHours}>{plural(draft.warnHours, 'hour')}</option>
					{/if}
				</select>
			</label>
		</SettingRow>
		<SettingRow title="Domain warning">
			<label class="flex flex-wrap items-center gap-2 text-sm">
				Flag a domain after
				<select
					bind:value={m.draft.stoppedDays}
					aria-label="Days without reports before a domain is flagged"
					class={[SELECT_CLASS, 'h-8']}
				>
					{#each STOPPED_DAYS as d (d)}
						<option value={d}>{d} days</option>
					{/each}
					{#if !STOPPED_DAYS.includes(draft.stoppedDays)}
						<option value={draft.stoppedDays}>{plural(draft.stoppedDays, 'day')}</option>
					{/if}
				</select>
				without reports
			</label>
		</SettingRow>
	</SettingsSection>
</SettingsPart>
