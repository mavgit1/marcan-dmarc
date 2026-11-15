<script lang="ts">
	import { goto } from '$app/navigation';
	import { resolve } from '$app/paths';
	import { page } from '$app/state';
	import Skeleton from '$lib/components/skeleton.svelte';
	import * as Select from '$lib/components/ui/select/index.js';
	import { filters, SINCE_OPTIONS, sinceLabel, syncScopeUrl } from '$lib/state/filters.svelte';
	import { refresh } from '$lib/state/refresh.svelte';
	import { scopeOptions } from '$lib/state/scope.svelte';
	import { scopeParamsFor } from '$lib/state/scope-url';
	import { cn } from '$lib/utils.js';

	let {
		class: className,
		ref = $bindable(null)
	}: {
		/** On the outer box: where the group sits in the header's row. */
		class?: string;
		/** The selects' own box, at its natural width (the header measures it). */
		ref?: HTMLElement | null;
	} = $props();

	const pathname = $derived(page.url.pathname);
	/**
	 * The organization and domain pickers only where the scope narrows the page (the lists and
	 * the dashboard). A page about one organization, domain or sender shows the time range alone:
	 * the trail says where it is.
	 */
	const params = $derived(scopeParamsFor(pathname));
	const showScopeSelectors = $derived(params.includes('org'));
	/**
	 * A page that reads no scope (Settings, the tools, a single report, the setup and its guide)
	 * shows no filter at all, not even the time range it would ignore. `scopeParamsFor` is the one
	 * list of what each page reads.
	 */
	const hideAll = $derived(params.length === 0);

	/** Ids and names only (shared store); a refresh keeps the last answer until the next arrives. */
	const scope = $derived(scopeOptions.current);
	let scopeFailed = $state(false);
	$effect(() => {
		if (!showScopeSelectors) return;
		void refresh.tick;
		scopeOptions.load().then(
			() => (scopeFailed = false),
			// Fail gracefully without blocking the header.
			() => (scopeFailed = scopeOptions.current === null)
		);
	});

	function onOrganizationChange(value: string) {
		filters.organizationId = value === 'all' ? null : Number(value);
		filters.domainId = null;
		void syncScopeUrl();
	}

	/**
	 * Narrows the list. A domain picked outside the alert list (which narrows in place) also
	 * opens its page: the list's URL takes the domain first, so Back returns to it narrowed.
	 */
	async function onDomainChange(value: string) {
		const next = value === 'all' ? null : Number(value);
		filters.domainId = next;
		await syncScopeUrl();
		if (next !== null && !pathname.startsWith('/alerts')) await goto(resolve(`/domains/${next}`));
	}

	function onSinceChange(value: string) {
		filters.since = value;
		void syncScopeUrl();
	}
</script>

{#if !hideAll}
	<div class={cn('flex justify-end', className)}>
		<div bind:this={ref} class="flex flex-wrap items-center justify-end gap-2">
			{#if showScopeSelectors && !scopeFailed}
				{#if scope === null}
					<Select.Root type="single" value={filters.organizationId?.toString() ?? 'all'} disabled>
						<Select.Trigger size="sm" class="min-w-[120px] max-w-[200px] w-auto" aria-label="Organization">
							<Skeleton class="h-3 w-20" />
						</Select.Trigger>
					</Select.Root>
					<Select.Root type="single" value="all" disabled>
						<Select.Trigger size="sm" class="min-w-[130px] max-w-[220px] w-auto" aria-label="Domain">
							<Skeleton class="h-3 w-24" />
						</Select.Trigger>
					</Select.Root>
				{:else}
					{@const data = scope}
					{#if data}
						{@const currentOrganizationValue = filters.organizationId?.toString() ?? 'all'}

						{@const organizationItems = [
							{ value: 'all', label: 'All organizations' },
							...data.organizations.map((c) => ({ value: String(c.id), label: c.name }))
						]}

						{@const availableDomains =
							currentOrganizationValue !== 'all'
								? data.domains.filter((d) => String(d.organization_id) === currentOrganizationValue)
								: data.domains}

						{@const currentDomainValue = filters.domainId?.toString() ?? 'all'}

						{@const domainItems = [
							{ value: 'all', label: 'All domains' },
							...availableDomains.map((d) => ({ value: String(d.id), label: d.domain }))
						]}

						{@const selectedOrganizationLabel =
							organizationItems.find((i) => i.value === currentOrganizationValue)?.label ?? 'All organizations'}
						<Select.Root
							type="single"
							items={organizationItems}
							value={currentOrganizationValue}
							onValueChange={onOrganizationChange}
						>
							<Select.Trigger size="sm" class="min-w-[120px] max-w-[200px] w-auto" aria-label="Organization">
								<span class="truncate block max-w-[170px]" title={selectedOrganizationLabel}>
									{selectedOrganizationLabel}
								</span>
							</Select.Trigger>
							<Select.Content class="min-w-[220px] max-w-[340px]">
								{#each organizationItems as item (item.value)}
									<Select.Item value={item.value} label={item.label}>
										<span class="truncate" title={item.label}>{item.label}</span>
									</Select.Item>
								{/each}
							</Select.Content>
						</Select.Root>

						{@const selectedDomainLabel =
							domainItems.find((i) => i.value === currentDomainValue)?.label ?? 'All domains'}
						<Select.Root type="single" items={domainItems} value={currentDomainValue} onValueChange={onDomainChange}>
							<Select.Trigger size="sm" class="min-w-[130px] max-w-[220px] w-auto" aria-label="Domain">
								<span class="truncate block max-w-[190px]" title={selectedDomainLabel}>
									{selectedDomainLabel}
								</span>
							</Select.Trigger>
							<Select.Content class="min-w-[240px] max-w-[360px]">
								{#each domainItems as item (item.value)}
									<Select.Item value={item.value} label={item.label}>
										<span class="truncate" title={item.label}>{item.label}</span>
									</Select.Item>
								{/each}
							</Select.Content>
						</Select.Root>
					{/if}
				{/if}
			{/if}

			<Select.Root type="single" items={[...SINCE_OPTIONS]} value={filters.since} onValueChange={onSinceChange}>
				<Select.Trigger size="sm" class="w-[130px]" aria-label="Time range">
					{sinceLabel(filters.since)}
				</Select.Trigger>
				<Select.Content>
					{#each SINCE_OPTIONS as item (item.value)}
						<Select.Item value={item.value} label={item.label}>{item.label}</Select.Item>
					{/each}
				</Select.Content>
			</Select.Root>
		</div>
	</div>
{/if}
