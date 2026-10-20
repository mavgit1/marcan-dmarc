<script lang="ts">
	import { createDomain } from '$lib/api';
	import { apiErrorMessage, conflictOrganizationName, isConflict } from '$lib/http';
	import { refresh } from '$lib/state/refresh.svelte';
	import { scopeOptions } from '$lib/state/scope.svelte';
	import BusyButton from '$lib/components/busy-button.svelte';
	import StatusBadge from '$lib/components/status-badge.svelte';
	import * as Select from '$lib/components/ui/select/index.js';
	import { listTokens, normalizeDomain, unique } from '$lib/text';
	import type { StatusTone } from '$lib/tokens';

	/**
	 * Multi-domain add form with per-domain results. With `organizationId` the target is fixed;
	 * without it an organization picker is shown, defaulting to the default organization.
	 */
	let {
		organizationId,
		onAdded,
		autofocus = false
	}: { organizationId?: number; onAdded?: () => void; autofocus?: boolean } = $props();

	type Outcome = { domain: string; tone: StatusTone; label: string; detail?: string };

	const uid = $props.id();

	let input = $state('');
	let busy = $state(false);
	let outcomes = $state.raw<Outcome[]>([]);
	let organizationsError = $state<string | null>(null);
	/** The user's choice; until then the default organization (first in the scope list). */
	let picked = $state<string>('');

	$effect(() => {
		if (organizationId !== undefined) return;
		void refresh.tick;
		scopeOptions.load().then(
			() => (organizationsError = null),
			(err: unknown) => (organizationsError = apiErrorMessage(err))
		);
	});

	const organizations = $derived(scopeOptions.current?.organizations ?? []);
	const pickedValue = $derived(
		organizations.some((c) => String(c.id) === picked) ? picked : organizations[0] ? String(organizations[0].id) : ''
	);
	const targetId = $derived(organizationId ?? (pickedValue ? Number(pickedValue) : null));
	const pickedName = $derived(organizations.find((c) => String(c.id) === pickedValue)?.name ?? '');

	/** Split on newlines, commas, semicolons and spaces; lowercase; drop duplicates. */
	const parse = (raw: string) => unique(listTokens(raw).map(normalizeDomain).filter(Boolean));

	const pending = $derived(parse(input));

	async function onSubmit(event: SubmitEvent) {
		event.preventDefault();
		if (busy || pending.length === 0 || targetId === null) return;
		busy = true;
		const results: Outcome[] = [];
		for (const domain of pending) {
			try {
				const created = await createDomain({ organization_id: targetId, domain });
				results.push(
					created.unignored
						? {
								domain,
								tone: 'pass',
								label: 'No longer ignored',
								detail: 'monitored again; its past reports are being re-imported'
							}
						: { domain, tone: 'pass', label: 'Added' }
				);
			} catch (err) {
				const message = apiErrorMessage(err);
				if (isConflict(err)) {
					const owner = conflictOrganizationName(err);
					results.push({
						domain,
						tone: 'neutral',
						label: 'Already exists',
						detail: owner ? `in ${owner}` : message
					});
				} else {
					results.push({ domain, tone: 'fail', label: 'Error', detail: message });
				}
			}
		}
		outcomes = results;
		// Keep only what failed in the box so it can be corrected and resubmitted.
		input = results
			.filter((r) => r.tone === 'fail')
			.map((r) => r.domain)
			.join('\n');
		busy = false;
		if (results.some((r) => r.tone === 'pass')) onAdded?.();
	}
</script>

<form class="space-y-3" onsubmit={onSubmit}>
	{#if organizationId === undefined}
		<div class="flex flex-wrap items-center gap-2">
			<span id="{uid}-org-label" class="text-muted-foreground text-xs font-medium">Organization</span>
			<Select.Root
				type="single"
				value={pickedValue}
				onValueChange={(v) => (picked = v)}
				disabled={busy || organizations.length === 0}
			>
				<Select.Trigger size="sm" class="w-auto max-w-[260px] min-w-[160px]" aria-labelledby="{uid}-org-label">
					<span class="block max-w-[220px] truncate">{pickedName || 'Select organization'}</span>
				</Select.Trigger>
				<Select.Content class="max-w-[340px] min-w-[200px]">
					{#each organizations as c (c.id)}
						<Select.Item value={String(c.id)} label={c.name}>
							<span class="truncate" title={c.name}>{c.name}</span>
						</Select.Item>
					{/each}
				</Select.Content>
			</Select.Root>
			{#if organizationsError}
				<span class="text-status-fail-fg text-xs">{organizationsError}</span>
			{/if}
		</div>
	{/if}
	<label for="{uid}-domains" class="sr-only">Domains, one per line</label>
	<textarea
		id="{uid}-domains"
		bind:value={input}
		rows="3"
		spellcheck="false"
		autocomplete="off"
		{@attach (el) => {
			// Focus after the opening click has settled, so the trigger button doesn't keep it.
			if (autofocus) setTimeout(() => el.focus());
		}}
		placeholder={'example.com\nexample.org'}
		class="border-input bg-transparent focus-visible:border-ring focus-visible:ring-ring/50 dark:bg-input/30 w-full rounded-lg border px-2.5 py-1.5 font-mono text-sm outline-none focus-visible:ring-3"
	></textarea>
	<div class="flex flex-wrap items-center justify-between gap-2">
		<p class="text-muted-foreground text-[11px]">One per line, or separated by commas.</p>
		<BusyButton type="submit" size="sm" {busy} busyLabel="Adding…" disabled={pending.length === 0 || targetId === null}>
			Add {pending.length > 1 ? `${pending.length} domains` : 'domain'}
		</BusyButton>
	</div>

	{#if outcomes.length > 0}
		<ul class="divide-border divide-y rounded-lg border text-sm" aria-live="polite">
			{#each outcomes as o (o.domain)}
				<li class="flex flex-wrap items-center justify-between gap-2 px-3 py-2">
					<span class="font-mono text-xs">{o.domain}</span>
					<span class="flex min-w-0 items-center gap-2">
						{#if o.detail}
							<span class="text-muted-foreground truncate text-[11px]" title={o.detail}>{o.detail}</span>
						{/if}
						<StatusBadge tone={o.tone} class="shrink-0 text-xs">{o.label}</StatusBadge>
					</span>
				</li>
			{/each}
		</ul>
	{/if}
</form>
