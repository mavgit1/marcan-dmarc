<script lang="ts">
	import { goto } from '$app/navigation';
	import { resolve } from '$app/paths';
	import { page } from '$app/state';
	import AddDomains from '$lib/components/add-domains.svelte';
	import BackgroundActivity from '$lib/components/background-activity.svelte';
	import DomainPortfolio from '$lib/components/domain-portfolio.svelte';
	import ExportDialog from '$lib/components/export-dialog.svelte';
	import PageHeader from '$lib/components/page-header.svelte';
	import { Button } from '$lib/components/ui/button/index.js';
	import * as Card from '$lib/components/ui/card/index.js';
	import { filters } from '$lib/state/filters.svelte';
	import { refresh } from '$lib/state/refresh.svelte';
	import PlusIcon from '@lucide/svelte/icons/plus';
	import XIcon from '@lucide/svelte/icons/x';

	let showAdd = $state(false);

	/** `?attention=1` (the dashboard's "more need attention" link): only the domains with an open issue. */
	const attention = $derived(page.url.searchParams.get('attention') === '1');

	/** Back to the plain list, keeping the other parameters (the scope). */
	function clearAttention() {
		const search = new URLSearchParams(page.url.searchParams);
		search.delete('attention');
		const query = search.toString();
		void goto(resolve(`/domains${query ? `?${query}` : ''}` as '/domains'), { keepFocus: true, noScroll: true });
	}
</script>

<PageHeader title="Domains">
	{#snippet actions()}
		<ExportDialog
			datasets={['domains', 'senders', 'auth-issues', 'dns-findings']}
			scope={{ organization_id: filters.organizationId, since: filters.since }}
		/>
		{#if !showAdd}
			<Button size="sm" class="gap-1.5" onclick={() => (showAdd = true)}>
				<PlusIcon class="size-3.5" />
				Add domains
			</Button>
		{/if}
	{/snippet}
</PageHeader>

<BackgroundActivity class="mb-4" />

{#if showAdd}
	<Card.Root class="mb-4">
		<Card.Header class="pb-3">
			<div class="flex items-center justify-between gap-2">
				<Card.Title class="text-sm font-semibold">Add domains</Card.Title>
				<Button variant="ghost" size="icon-sm" aria-label="Close" onclick={() => (showAdd = false)}>
					<XIcon class="size-3.5" />
				</Button>
			</div>
		</Card.Header>
		<Card.Content>
			<AddDomains autofocus onAdded={() => refresh.refresh()} />
		</Card.Content>
	</Card.Root>
{/if}

<DomainPortfolio
	organizationId={filters.organizationId}
	showOrganization={filters.organizationId == null}
	{attention}
	onClearAttention={clearAttention}
	emptyDescription="Add one above. Reports are optional: DNS posture is visible from the first check."
/>
