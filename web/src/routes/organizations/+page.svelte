<script lang="ts">
	import { goto } from '$app/navigation';
	import { resolve } from '$app/paths';
	import { createOrganization, getOrganizations } from '$lib/api';
	import type { Items } from '$lib/types/Items';
	import type { OrganizationRow } from '$lib/types/OrganizationRow';
	import OutcomeBar from '$lib/components/outcome-bar.svelte';
	import StatusBadge from '$lib/components/status-badge.svelte';
	import { filters } from '$lib/state/filters.svelte';
	import { formatInt, plural } from '$lib/format';
	import { resource } from '$lib/state/resource.svelte';
	import { statusFg } from '$lib/tokens';
	import EmptyState from '$lib/components/empty-state.svelte';
	import Loaded from '$lib/components/loaded.svelte';
	import PageHeader from '$lib/components/page-header.svelte';
	import Skeleton from '$lib/components/skeleton.svelte';
	import { Action } from '$lib/components/action.svelte';
	import BusyButton from '$lib/components/busy-button.svelte';
	import SettingsLink from '$lib/components/settings/settings-link.svelte';
	import FeedbackAlert from '$lib/components/settings/feedback-alert.svelte';
	import { Button } from '$lib/components/ui/button/index.js';
	import { Input } from '$lib/components/ui/input/index.js';
	import { refresh } from '$lib/state/refresh.svelte';
	import Building2Icon from '@lucide/svelte/icons/building-2';
	import PlusIcon from '@lucide/svelte/icons/plus';

	let organizationName = $state('');
	let showForm = $state(false);
	const create = new Action();

	const organizations = resource({
		key: () => filters.since,
		load: () => getOrganizations({ since: filters.since })
	});

	function onCreate(event: SubmitEvent) {
		event.preventDefault();
		const name = organizationName.trim();
		if (!name || create.busy) return;
		void create.run(async () => {
			const created = await createOrganization({ name });
			organizationName = '';
			showForm = false;
			refresh.refresh();
			await goto(resolve(`/organizations/${created.id}`));
		});
	}
</script>

<PageHeader title="Organizations">
	{#snippet below()}
		<p class="text-muted-foreground max-w-2xl text-sm">
			New domains land in the default organization. To move many at once, edit the
			<SettingsLink to="domains">domain list</SettingsLink> in Settings.
		</p>
	{/snippet}
	{#snippet actions()}
		{#if showForm}
			<form class="flex flex-wrap items-center gap-2" onsubmit={onCreate}>
				<Input
					bind:value={organizationName}
					placeholder="Organization name"
					autocomplete="off"
					spellcheck={false}
					class="h-8 w-56 text-xs"
					aria-label="Organization name"
					autofocus
				/>
				<BusyButton
					type="submit"
					size="sm"
					class="h-8"
					busy={create.busy}
					busyLabel="Creating…"
					disabled={!organizationName.trim()}
				>
					Create organization
				</BusyButton>
				<Button
					type="button"
					variant="ghost"
					size="sm"
					class="h-8 text-xs"
					disabled={create.busy}
					onclick={() => {
						showForm = false;
						create.feedback = null;
					}}
				>
					Cancel
				</Button>
			</form>
		{:else}
			<Button size="sm" class="gap-1.5" onclick={() => (showForm = true)}>
				<PlusIcon class="size-3.5" />
				New organization
			</Button>
		{/if}
	{/snippet}
</PageHeader>

{#if create.feedback && !create.feedback.ok}
	<div class="mb-4">
		<FeedbackAlert feedback={create.feedback} />
	</div>
{/if}

{#snippet list(data: Items<OrganizationRow>)}
	{#if data.items.length === 0}
		<EmptyState
			icon={Building2Icon}
			headline="No organizations yet"
			description="Create an organization or import domains from aggregate reports."
		>
			{#snippet action()}
				<Button size="sm" class="gap-1.5" onclick={() => (showForm = true)}>
					<PlusIcon class="size-3.5" />
					New organization
				</Button>
			{/snippet}
		</EmptyState>
	{:else}
		<!--
			Stacked while the list is narrow (a phone). From @xl every row is the same grid, each
			cell rendered even when empty, so the columns line up: the name truncates, the rest
			have fixed widths, and the counts go under them until @4xl gives them a column. The
			list's own width decides, not the viewport's: the sidebar takes a share from md up.
		-->
		<ul class="divide-border @container divide-y rounded-lg border text-sm">
			{#each data.items as c (c.id)}
				{@const attention = c.attention}
				<li>
					<a
						href={resolve(`/organizations/${c.id}`)}
						class="hover:bg-muted/50 flex flex-wrap items-center gap-x-4 gap-y-1.5 px-3 py-2.5 transition-colors @xl:grid @xl:grid-cols-[minmax(0,1fr)_9.5rem_7rem_6rem] @xl:gap-x-3 @4xl:grid-cols-[minmax(0,1fr)_9.5rem_7rem_8rem_18rem]"
					>
						<span class="flex w-full min-w-0 items-center gap-2 @xl:w-auto">
							<span class="truncate font-medium hover:underline" title={c.name}>{c.name}</span>
							{#if c.is_default}
								<StatusBadge tone="neutral" class="text-[10px]">
									<span title="Domains discovered in reports are placed here">Default</span>
								</StatusBadge>
							{/if}
						</span>
						<span
							class={[
								'text-xs',
								statusFg[attention.status],
								attention.domains_needing_attention === 0 && 'hidden @xl:block'
							]}
						>
							{#if attention.domains_needing_attention > 0}
								{formatInt(attention.domains_needing_attention)} of {formatInt(c.domain_count)} need attention
							{/if}
						</span>
						<span class={['text-muted-foreground text-xs', !attention.worst_dns && 'hidden @xl:block']}>
							{#if attention.worst_dns}
								Worst DNS grade
								<span class={['font-medium', statusFg[attention.worst_dns.status]]}>{attention.worst_dns.grade}</span>
							{/if}
						</span>
						<!-- A phone shows the bar full width under the meta line instead of hiding it. -->
						<OutcomeBar class="order-last w-full @xl:order-none @xl:w-auto" mail={c} />
						<span
							class="text-muted-foreground text-xs tabular-nums @xl:col-span-full @xl:row-start-2 @4xl:col-span-1 @4xl:row-start-auto @4xl:text-right @4xl:whitespace-nowrap"
						>
							{plural(c.domain_count, 'domain')} · {plural(c.report_count, 'report')} ·
							{plural(c.total_messages, 'message')}
						</span>
					</a>
				</li>
			{/each}
		</ul>
	{/if}
{/snippet}

{#snippet loading()}
	<div class="divide-border divide-y rounded-lg border">
		{#each { length: 3 }}
			<div class="flex items-center justify-between gap-4 px-3 py-2.5">
				<Skeleton class="w-40" />
				<Skeleton class="w-28" />
			</div>
		{/each}
	</div>
{/snippet}

<Loaded source={organizations} errorTitle="Failed to load organizations" data={list} {loading} />
