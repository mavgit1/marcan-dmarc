<script lang="ts" module>
	import { type ExportDataset } from '$lib/api';

	const DATASETS: Record<ExportDataset, { label: string; description: string }> = {
		domains: {
			label: 'Domains',
			description: 'One row per domain: organization, DNS grade, policy and message totals.'
		},
		senders: {
			label: 'Senders',
			description: 'One row per sending IP and domain: messages and authentication results.'
		},
		'auth-issues': {
			label: 'Authentication issues',
			description: 'Report records where SPF or DKIM did not pass for your domain, with outcome and reporter.'
		},
		'dns-findings': {
			label: 'DNS findings',
			description: 'Findings of the latest DNS check per domain, with severity and suggestion.'
		}
	};
</script>

<script lang="ts">
	import { EXPORT_PARAMS, exportCsvUrl, type ExportScope } from '$lib/api';
	import { failureKindsScope } from '$lib/wording/outcome';
	import { Button } from '$lib/components/ui/button/index.js';
	import * as Popover from '$lib/components/ui/popover/index.js';
	import { sinceLabel } from '$lib/state/filters.svelte';
	import { scopeOptions } from '$lib/state/scope.svelte';
	import { cn } from '$lib/utils.js';
	import DownloadIcon from '@lucide/svelte/icons/download';

	/**
	 * The one export control of a page: pick a dataset, see the scope it covers (the page's
	 * current filters, read-only), download. Offer only the datasets that fit the page.
	 */
	let {
		datasets,
		scope,
		initial,
		organizationName,
		domainName
	}: {
		datasets: ExportDataset[];
		scope: ExportScope;
		/** Preselected dataset (e.g. auth issues from the Failures tab). */
		initial?: ExportDataset;
		/** Shown in the scope line; looked up from `scope` when omitted. */
		organizationName?: string | null;
		domainName?: string | null;
	} = $props();

	let open = $state(false);
	let picked = $state<ExportDataset | null>(null);
	const dataset = $derived(picked && datasets.includes(picked) ? picked : (initial ?? datasets[0]));

	/** Names for ids the page did not label, from the shared scope list (loaded on open). */
	$effect(() => {
		if (!open) return;
		const needOrganization = scope.organization_id != null && organizationName == null;
		const needDomain = scope.domain_id != null && domainName == null;
		if (!needOrganization && !needDomain) return;
		scopeOptions.load().catch(() => {
			/* fall back to ids */
		});
	});
	const names = $derived.by(() => {
		const s = scopeOptions.current;
		return {
			organizations: new Map((s?.organizations ?? []).map((c) => [c.id, c.name])),
			domains: new Map((s?.domains ?? []).map((d) => [d.id, d.domain]))
		};
	});

	/** What the download covers, using only the parameters this dataset takes. */
	const scopeParts = $derived.by(() => {
		const keys = EXPORT_PARAMS[dataset];
		const parts: string[] = [];
		if (keys.includes('organization_id')) {
			if (scope.organization_id != null) {
				const name = organizationName ?? names.organizations.get(scope.organization_id) ?? `#${scope.organization_id}`;
				parts.push(`Organization ${name}`);
			} else if (!(keys.includes('domain_id') && scope.domain_id != null)) {
				parts.push('All organizations');
			}
		}
		if (keys.includes('domain_id')) {
			if (scope.domain_id != null) {
				parts.push(domainName ?? names.domains.get(scope.domain_id) ?? `Domain #${scope.domain_id}`);
			} else {
				parts.push('All domains');
			}
		}
		if (keys.includes('since')) parts.push(sinceLabel(scope.since));
		if (keys.includes('outcome')) {
			const kinds = failureKindsScope(scope.outcome);
			if (kinds) parts.push(kinds);
		}
		if (keys.includes('source_ip') && scope.source_ip?.trim()) {
			parts.push(`IP ${scope.source_ip.trim()}`);
		}
		if (!keys.includes('since')) parts.push('latest check');
		return parts;
	});

	const href = $derived(exportCsvUrl(dataset, scope));
</script>

<Popover.Root bind:open>
	<Popover.Trigger>
		{#snippet child({ props })}
			<Button {...props} variant="outline" size="sm" class="gap-1.5">
				<DownloadIcon class="size-3.5" />
				Export
			</Button>
		{/snippet}
	</Popover.Trigger>
	<Popover.Content class="w-80 space-y-3">
		<div class="space-y-0.5">
			<p class="text-sm font-medium">Export CSV</p>
			<p class="text-muted-foreground text-xs">Everything in scope, no row limit.</p>
		</div>

		<fieldset class="space-y-1">
			<legend class="sr-only">Dataset</legend>
			{#each datasets as id (id)}
				<label
					class={cn(
						'hover:bg-muted/60 flex cursor-pointer items-start gap-2 rounded-md border border-transparent px-2 py-1.5',
						dataset === id && 'border-border bg-muted/60'
					)}
				>
					<input
						type="radio"
						name="export-dataset"
						class="accent-primary mt-0.5"
						value={id}
						checked={dataset === id}
						onchange={() => (picked = id)}
					/>
					<span class="min-w-0 space-y-0.5">
						<span class="block text-sm font-medium">{DATASETS[id].label}</span>
						<span class="text-muted-foreground block text-xs">{DATASETS[id].description}</span>
					</span>
				</label>
			{/each}
		</fieldset>

		<p class="text-muted-foreground text-xs" data-testid="export-scope">
			<span class="text-foreground font-medium">Scope:</span>
			{scopeParts.join(' · ')}
		</p>

		<Button {href} download={`${dataset}.csv`} size="sm" class="w-full gap-1.5" onclick={() => (open = false)}>
			<DownloadIcon class="size-3.5" />
			Download {DATASETS[dataset].label.toLowerCase()}
		</Button>
	</Popover.Content>
</Popover.Root>
