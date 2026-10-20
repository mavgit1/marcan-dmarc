<script lang="ts">
	import Skeleton from '$lib/components/skeleton.svelte';
	import { Input } from '$lib/components/ui/input/index.js';
	import { meta } from '$lib/state/meta.svelte';
	import { catalogueSummary, groupCatalogue } from '$lib/wording/alert-catalogue';
	import { statusText } from '$lib/tokens';
	import type { DigestFrequency } from '$lib/types/DigestFrequency';
	import type { Weekday } from '$lib/types/Weekday';
	import ChevronRightIcon from '@lucide/svelte/icons/chevron-right';
	import BusyButton from '../busy-button.svelte';
	import { AlertsModel, DIGEST_LABEL, WEEKDAY_LABEL } from './alerts-draft.svelte';
	import { onFocusAnchor } from '$lib/state/anchor-focus.svelte';
	import FeedbackAlert from './feedback-alert.svelte';
	import HourInput from './hour-input.svelte';
	import { joinPageForm } from './page-form.svelte';
	import SettingRow from './setting-row.svelte';
	import SettingsPart from './settings-part.svelte';
	import SettingsSection from './settings-section.svelte';
	import TestRecipientLine from './test-recipient-line.svelte';

	/**
	 * The Alerts page's sections over the shared settings (the alerts group): Delivery (the
	 * recipients, the digest and its time, the base URL for links), Alert kinds (full width: per
	 * group, each kind with an Email checkbox; folded to one line until opened) and Test; saved by
	 * the page's save bar. Whether alert emails go out at all is the page's switch (`SettingsPage`'s
	 * `feature`), which disables them while it is off.
	 */
	let { model: m, idPrefix = 'alerts' }: { model: AlertsModel; idPrefix?: string } = $props();

	joinPageForm(() => m);

	$effect(() => {
		meta.load().catch(() => {});
	});
	/** The server's alert catalogue by group ("Alert kinds"). */
	const catalogue = $derived(
		meta.current ? groupCatalogue(meta.current.alert_catalogue, meta.current.alert_groups) : null
	);
	/** The draft's choice in one line, shown while the catalogue is folded. */
	const catalogueLine = $derived(
		meta.current ? catalogueSummary(meta.current.alert_catalogue, m.draft.muted_kinds) : null
	);

	/** "Alert kinds" is folded until opened, or until a link to it (`alert-kinds`) is followed. */
	let catalogueOpen = $state(false);
	let catalogueEl = $state<HTMLElement | null>(null);
	onFocusAnchor(
		() => 'alert-kinds',
		() => catalogueEl,
		() => (catalogueOpen = true)
	);

	const DIGESTS = Object.keys(DIGEST_LABEL) as DigestFrequency[];
	const WEEKDAYS = Object.keys(WEEKDAY_LABEL) as Weekday[];

	const id = (name: string) => `${idPrefix}-${name}`;
	const selectClass =
		'border-input bg-background h-9 min-w-0 rounded-md border px-2 text-sm shadow-xs disabled:opacity-50';
</script>

{#snippet label(forId: string, text: string)}
	<label for={forId} class="text-muted-foreground text-xs font-medium">{text}</label>
{/snippet}

{#snippet note(text: string, tone: 'neutral' | 'warn' = 'neutral')}
	<p class={['mt-1 text-[11px]', statusText[tone]]}>{text}</p>
{/snippet}

<SettingsPart form={m} errorTitle="Alert settings failed to load" skeleton="h-72">
	<SettingsSection title="Delivery">
		<SettingRow title="Recipients" hint="One address per line.">
			<textarea
				id={id('recipients')}
				bind:value={m.draft.recipients}
				rows="3"
				placeholder="dmarc-team@example.com"
				aria-label="Recipients"
				class="border-input bg-background w-full max-w-md rounded-md border px-3 py-2 text-sm shadow-xs"></textarea>
			{#if m.badRecipient}
				{@render note(`${m.badRecipient} is not a valid email address.`, 'warn')}
			{/if}
		</SettingRow>

		<SettingRow title="Digest" hint="Every open issue and what changed, also on quiet days.">
			<div class="flex flex-wrap items-end gap-3">
				<div>
					{@render label(id('digest'), 'Frequency')}
					<select id={id('digest')} bind:value={m.draft.digest} class={['mt-1 block', selectClass]}>
						{#each DIGESTS as digest (digest)}
							<option value={digest}>{DIGEST_LABEL[digest]}</option>
						{/each}
					</select>
				</div>
				{#if m.draft.digest === 'weekly'}
					<div>
						{@render label(id('weekday'), 'Day')}
						<select id={id('weekday')} bind:value={m.draft.digest_weekday} class={['mt-1 block', selectClass]}>
							{#each WEEKDAYS as day (day)}
								<option value={day}>{WEEKDAY_LABEL[day]}</option>
							{/each}
						</select>
					</div>
				{/if}
				<div>
					{@render label(id('hour'), 'Time (UTC)')}
					<HourInput
						id={id('hour')}
						bind:value={m.draft.digest_hour}
						disabled={m.draft.digest === 'off'}
						class="mt-1 block"
					/>
				</div>
			</div>
		</SettingRow>

		<SettingRow title="Base URL" hint="Marcan's own address, for links in emails. Empty: no links.">
			<Input
				id={id('base-url')}
				type="url"
				bind:value={m.draft.base_url}
				placeholder="https://marcan.example.com"
				aria-label="Base URL"
				class="max-w-md text-sm"
			/>
		</SettingRow>
	</SettingsSection>

	<div id="alert-kinds" bind:this={catalogueEl} class="scroll-mt-below-header">
		<SettingsSection title="Alert kinds">
			<!-- Full width, not a setting row: the kinds are a list to read, each with its own switch. -->
			<div class="min-w-0 px-5 py-4">
				<button
					type="button"
					class="hover:text-foreground inline-flex items-center gap-1.5 text-sm"
					aria-expanded={catalogueOpen}
					aria-controls={id('catalogue')}
					onclick={() => (catalogueOpen = !catalogueOpen)}
				>
					<ChevronRightIcon
						class={['text-muted-foreground size-4 shrink-0 transition-transform', catalogueOpen && 'rotate-90']}
					/>
					<span class="tabular-nums">{catalogueLine ?? '…'}</span>
				</button>
				<div id={id('catalogue')} class={['mt-2', !catalogueOpen && 'hidden']}>
					{#if catalogue}
						{#each catalogue as g (g.group)}
							{@const ids = g.entries.map((e) => e.id)}
							<div role="group" aria-labelledby={id(`group-${g.group}`)} class="mt-4 min-w-0">
								<div class="flex flex-wrap items-baseline justify-between gap-x-3 gap-y-1 border-b pb-1.5">
									<h4 id={id(`group-${g.group}`)} class="text-muted-foreground text-xs font-semibold">
										{g.title}
									</h4>
									{#if ids.length > 1}
										<span class="flex gap-3">
											<button
												type="button"
												class="text-muted-foreground hover:text-foreground text-xs underline underline-offset-2"
												onclick={() => m.setMailed(ids, true)}>Email all</button
											>
											<button
												type="button"
												class="text-muted-foreground hover:text-foreground text-xs underline underline-offset-2"
												onclick={() => m.setMailed(ids, false)}>Email none</button
											>
										</span>
									{/if}
								</div>
								<ul class="divide-border divide-y">
									{#each g.entries as entry (entry.id)}
										{@const box = id(`email-${entry.id}`)}
										<li class="flex items-start justify-between gap-4 py-2.5">
											<div class="min-w-0">
												<span id={`${box}-title`} class="block text-sm">{entry.title}</span>
												<span class="text-muted-foreground block text-xs leading-relaxed">{entry.trigger}</span>
											</div>
											<label class="flex shrink-0 cursor-pointer items-center gap-2 text-sm">
												<input
													id={box}
													type="checkbox"
													checked={m.isMailed(entry.id)}
													onchange={(e) => m.setMailed([entry.id], e.currentTarget.checked)}
													aria-labelledby={`${box}-label ${box}-title`}
													class="accent-primary size-4"
												/>
												<span id={`${box}-label`}>Email</span>
											</label>
										</li>
									{/each}
								</ul>
							</div>
						{/each}
					{:else}
						<Skeleton variant="block" class="mt-4 h-40" />
					{/if}
				</div>
			</div>
		</SettingsSection>
	</div>

	<SettingsSection title="Test">
		<SettingRow title="Test alert">
			<BusyButton
				type="button"
				variant="outline"
				size="sm"
				busy={m.test.busy}
				busyLabel="Sending…"
				disabled={!!m.testBlocked}
				onclick={() => m.sendTest()}
			>
				Send a test
			</BusyButton>
			{#if m.testBlocked && !m.test.busy}
				{@render note(m.testBlocked)}
			{:else}
				<TestRecipientLine to={m.testRecipient} />
			{/if}
			{#if m.test.feedback}
				<div class="mt-3 max-w-md">
					<FeedbackAlert feedback={m.test.feedback} failTitle="The test alert was not sent" />
				</div>
			{/if}
		</SettingRow>
	</SettingsSection>
</SettingsPart>
