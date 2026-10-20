<script lang="ts">
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
	emptyDescription="Add one above. Reports are optional: DNS posture is visible from the first check."
/>
