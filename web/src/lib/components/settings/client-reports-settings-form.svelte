<script lang="ts">
	import { Input } from '$lib/components/ui/input/index.js';
	import { meta } from '$lib/state/meta.svelte';
	import { statusText } from '$lib/tokens';
	import type { RecipientsSummary } from '$lib/types/RecipientsSummary';
	import { ClientReportsModel, dayRange } from './client-reports-draft.svelte';
	import HourInput from './hour-input.svelte';
	import type { ListDraft } from './list-draft.svelte';
	import ListEditor from './list-editor.svelte';
	import SettingsLink from './settings-link.svelte';
	import { joinPageForm } from './page-form.svelte';
	import SettingRow from './setting-row.svelte';
	import SettingsPart from './settings-part.svelte';
	import SettingsSection from './settings-section.svelte';

	/**
	 * The Client reports page's sections over the shared settings (the client reports group):
	 * Schedule (the day and hour, the Reply-To), then Recipients (who gets them, the `recipients`
	 * list, a part of its own). The Preview is `ClientReportsPreview`, outside the rows the switch
	 * disables. Switched on,
	 * the group is saved after the recipients (`saveLast`): the server needs the list saved first.
	 * Switched off, before them: the server keeps the list from being emptied while the reports
	 * are on.
	 */
	let {
		model: m,
		idPrefix = 'client-reports',
		recipients
	}: { model: ClientReportsModel; idPrefix?: string; recipients: ListDraft<RecipientsSummary> } = $props();

	joinPageForm(() => m);

	$effect(() => {
		meta.load().catch(() => {});
	});
	/** The days issues and the next step are judged on: policy readiness's window, served by `meta`. */
	const windowDays = $derived(meta.current?.readiness_window_days ?? '…');

	const id = (name: string) => `${idPrefix}-${name}`;
</script>

{#snippet label(forId: string, text: string)}
	<label for={forId} class="text-muted-foreground text-xs font-medium">{text}</label>
{/snippet}

{#snippet note(text: string, tone: 'neutral' | 'warn' = 'neutral')}
	<p class={['mt-1 text-[11px]', statusText[tone]]}>{text}</p>
{/snippet}

<SettingsPart form={m} errorTitle="Client report settings failed to load" skeleton="h-56">
	{#snippet children(saved)}
		<SettingsSection title="Schedule">
			<SettingRow title="When" hint="Covers the month before; issues are judged on the last {windowDays} days.">
				<div class="flex flex-wrap items-end gap-3">
					<div>
						{@render label(id('day'), 'Day of the month')}
						<Input
							id={id('day')}
							type="number"
							min={dayRange(meta.current)?.min}
							max={dayRange(meta.current)?.max}
							bind:value={m.draft.day_of_month}
							aria-invalid={m.ariaInvalid('day_of_month')}
							class="mt-1 block h-9 w-20 text-sm tabular-nums"
						/>
					</div>
					<div>
						{@render label(id('hour'), 'Time (UTC)')}
						<HourInput id={id('hour')} bind:value={m.draft.hour_utc} class="mt-1 block" />
					</div>
				</div>
				{#if m.dayProblem}
					{@render note(m.dayProblem, 'warn')}
				{/if}
			</SettingRow>

			<SettingRow title="Reply-To">
				<!-- Empty: the Reply-To under Sending, else the sender address; the placeholder names it. -->
				<Input
					id={id('reply-to')}
					type="email"
					bind:value={m.draft.reply_to}
					aria-invalid={m.ariaInvalid('reply_to')}
					placeholder={saved.sending.reply_to
						? `Same as under Sending: ${saved.sending.reply_to}`
						: 'Same as the sender address'}
					aria-label="Reply-To"
					class="max-w-md text-sm"
				/>
				{#if m.problem && !m.dayProblem}
					{@render note(m.problem, 'warn')}
				{/if}
			</SettingRow>
		</SettingsSection>

		<ListEditor
			list={recipients}
			invalid={m.fieldProblem('recipients')}
			title="Recipients"
			anchor="client-report-recipients"
			example={'Acme,it@acme.example\nAcme,ceo@acme.example\nGlobex,dmarc@globex.example\nInitech,none'}
			docs="client-reports#who-gets-it"
		>
			{#snippet format()}
				<li>
					One <code class="font-mono">organization,email</code> per line, for an organization that exists (add it under
					<SettingsLink to="domains" class="text-inherit" />). Organizations you leave out stay as they are.
				</li>
				<li class={statusText.warn}>
					An organization you list gets exactly its listed addresses; <code class="font-mono">,none</code> stops its reports.
				</li>
			{/snippet}
		</ListEditor>
	{/snippet}
</SettingsPart>
