<script lang="ts">
	import { getDkimSelectors, previewDkimSelectors, saveDkimSelectors } from '$lib/api';
	import type { DkimCheckResult } from '$lib/types/DkimCheckResult';
	import DnsRawRecord from '$lib/components/dns-raw-record.svelte';
	import { Action } from '$lib/components/action.svelte';
	import BusyButton from '$lib/components/busy-button.svelte';
	import FeedbackAlert from '$lib/components/settings/feedback-alert.svelte';
	import StatusBadge from '$lib/components/status-badge.svelte';
	import { dkimKey, dkimSelectorOrigin, selectorVerdict } from '$lib/dns';
	import { formatInt, plural } from '$lib/format';
	import { meta } from '$lib/state/meta.svelte';
	import { statusFg } from '$lib/tokens';
	import DetailHeading from './detail-heading.svelte';

	let {
		check,
		onRefresh
	}: {
		check: DkimCheckResult | undefined;
		onRefresh?: () => void;
	} = $props();

	// Selectors seen in reports first, then published keys, then the rest.
	const records = $derived(
		[...(check?.records ?? [])].sort(
			(a, b) =>
				Number(b.seen_in_reports ?? false) - Number(a.seen_in_reports ?? false) ||
				Number(b.present) - Number(a.present) ||
				a.selector.localeCompare(b.selector)
		)
	);
	const published = $derived(records.filter((r) => r.present && r.raw));

	$effect(() => {
		meta.load().catch(() => {});
	});
	const windowDays = $derived(meta.current?.observed_selector_window_days);

	let draft = $state('');
	const add = new Action();

	function addSelector(e: Event) {
		e.preventDefault();
		const sel = draft.trim();
		if (!sel || add.busy) return;
		void add.run(async () => {
			// The stored list with this selector added, previewed and saved like Settings saves it.
			const list = `${(await getDkimSelectors()).text}${sel}\n`;
			const [error] = (await previewDkimSelectors(list)).errors;
			if (error) throw new Error(error.message);
			await saveDkimSelectors(list);
			draft = '';
			onRefresh?.();
		});
	}
</script>

<div class="space-y-4">
	<div>
		<div class="flex items-center justify-between mb-2">
			<DetailHeading>Selectors checked</DetailHeading>
			{#if records.length > 0}
				<span class="text-xs text-muted-foreground tabular-nums">
					{check?.discovered_count ?? 0} of {records.length} published
				</span>
			{/if}
		</div>

		{#if records.length > 0}
			<div class="overflow-x-auto rounded-lg border border-border">
				<table class="w-full text-left text-xs">
					<thead class="border-b border-border bg-muted/40 font-medium text-muted-foreground">
						<tr>
							<th class="p-2.5">Selector</th>
							<th class="p-2.5">Key</th>
							<th class="p-2.5">Source</th>
							<th class="p-2.5">In reports (verified / failed)</th>
							<th class="p-2.5">Status</th>
						</tr>
					</thead>
					<tbody class="divide-y divide-border tabular-nums">
						{#each records as rec (rec.selector)}
							{@const key = dkimKey(rec)}
							{@const status = selectorVerdict(rec.verdict)}
							<tr class="hover:bg-muted/10">
								<td class="p-2.5 font-mono font-medium text-foreground">{rec.selector}</td>
								<td class="p-2.5">
									{#if rec.present}
										<span class={statusFg[key.tone]}>{key.label}</span>
									{:else}
										<span class="text-muted-foreground">—</span>
									{/if}
								</td>
								<td class="p-2.5 text-muted-foreground">{dkimSelectorOrigin(rec)}</td>
								<td class="p-2.5">
									{#if rec.report_pass_messages != null || rec.report_fail_messages != null}
										{formatInt(rec.report_pass_messages ?? 0)} / {formatInt(rec.report_fail_messages ?? 0)}
									{:else}
										<span class="text-muted-foreground">—</span>
									{/if}
								</td>
								<td class="p-2.5">
									<StatusBadge tone={status.tone}>{status.label}</StatusBadge>
								</td>
							</tr>
						{/each}
					</tbody>
				</table>
			</div>
			<p class="text-muted-foreground mt-2 text-[11px]">
				Selectors seen in reports{windowDays != null ? ` (last ${plural(windowDays, 'day')})` : ''} plus your selector list.
			</p>
		{:else}
			<div class="rounded-md border border-border bg-muted/10 p-3 text-xs text-muted-foreground">
				No selectors: none seen in reports and your selector list is empty.
			</div>
		{/if}
	</div>

	{#if published.length > 0}
		<div class="space-y-3">
			<DetailHeading>Published DKIM records</DetailHeading>
			{#each published as rec (rec.selector)}
				<DnsRawRecord value={rec.raw ?? ''} label={`Selector ${rec.selector}`} />
			{/each}
		</div>
	{/if}

	<div
		class="flex flex-col sm:flex-row sm:items-center justify-between gap-2.5 rounded-lg border border-border bg-card p-3"
	>
		<div>
			<p class="text-xs text-muted-foreground">Add a selector; it is checked on every domain.</p>
			{#if add.feedback && !add.feedback.ok}
				<div class="mt-1.5">
					<FeedbackAlert feedback={add.feedback} failTitle="Failed to add selector" />
				</div>
			{/if}
		</div>
		<form onsubmit={addSelector} class="flex items-center gap-1.5">
			<input
				type="text"
				bind:value={draft}
				placeholder="Selector"
				aria-label="DKIM selector"
				class="h-7 w-36 sm:w-44 rounded-md border border-input bg-background px-2 text-xs font-mono placeholder:font-sans placeholder:text-muted-foreground focus-visible:outline-none focus-visible:ring-1 focus-visible:ring-ring"
				disabled={add.busy}
			/>
			<BusyButton
				type="submit"
				size="sm"
				variant="outline"
				class="h-7 px-2.5 text-xs font-medium gap-1"
				busy={add.busy}
				busyLabel="Adding…"
				disabled={!draft.trim()}
			>
				Add selector
			</BusyButton>
		</form>
	</div>
</div>
