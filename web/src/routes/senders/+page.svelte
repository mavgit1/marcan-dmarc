<script lang="ts">
	import ExportDialog from '$lib/components/export-dialog.svelte';
	import PageHeader from '$lib/components/page-header.svelte';
	import SendersTable from '$lib/components/senders-table.svelte';
	import { filters } from '$lib/state/filters.svelte';
</script>

<PageHeader title="Senders" description="Source IPs seen in this scope, busiest first.">
	{#snippet actions()}
		<ExportDialog
			datasets={['senders', 'auth-issues']}
			scope={{ organization_id: filters.organizationId, domain_id: filters.domainId, since: filters.since }}
		/>
	{/snippet}
</PageHeader>

<SendersTable />
