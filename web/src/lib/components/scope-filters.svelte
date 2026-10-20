<script lang="ts">
	import { goto } from '$app/navigation';
	import { resolve } from '$app/paths';
	import { page } from '$app/state';
	import type { ScopeDomain } from '$lib/types/ScopeDomain';
	import Skeleton from '$lib/components/skeleton.svelte';
	import * as Select from '$lib/components/ui/select/index.js';
	import { filters, scopedHref, SINCE_OPTIONS, sinceLabel, syncScopeUrl } from '$lib/state/filters.svelte';
	import { refresh } from '$lib/state/refresh.svelte';
	import { scopeOptions } from '$lib/state/scope.svelte';
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
	const hideScopeSelectors = $derived(pathname.startsWith('/settings') || pathname.startsWith('/tools'));
	/** The setup and its guide show no scoped data, so no filter applies there. */
	const hideAll = $derived(pathname.startsWith('/guide') || pathname.startsWith('/setup'));

	/** Ids and names only (shared store); a refresh keeps the last answer until the next arrives. */
	const scope = $derived(scopeOptions.current);
	let scopeFailed = $state(false);
	$effect(() => {
		if (hideScopeSelectors) return;
		void refresh.tick;
		scopeOptions.load().then(
			() => (scopeFailed = false),
			// Fail gracefully without blocking the header.
			() => (scopeFailed = scopeOptions.current === null)
		);
	});

	/**
	 * An organization page shows its organization because of its route: there the filters open
	 * another page and leave the stored scope alone (a domain's page with `?org=`, as the page's
	 * own links do).
	 */
	const onOrganizationPage = $derived(pathname.startsWith('/organizations/') && Boolean(page.params.id));

	async function onOrganizationChange(value: string, allDomains: ScopeDomain[]) {
		const next = value === 'all' ? null : Number(value);
		if (onOrganizationPage) {
			await goto(resolve(next !== null ? `/organizations/${next}` : '/organizations'));
			return;
		}
		filters.organizationId = next;
		filters.domainId = null;

		if (pathname.startsWith('/domains/')) {
			if (next !== null) {
				// Stay on a domain of this organization; otherwise open the organization
				const currentDomain = allDomains.find((d) => String(d.id) === page.params.id);
				if (!currentDomain || currentDomain.organization_id !== next) {
					await goto(resolve(`/organizations/${next}`));
				}
			} else {
				await goto(resolve('/domains'));
			}
		}
		syncScopeUrl();
	}

	async function onDomainChange(value: string) {
		const next = value === 'all' ? null : Number(value);
		if (onOrganizationPage) {
			if (next !== null) {
				const href = scopedHref(`/domains/${next}`, { organizationId: Number(page.params.id) });
				await goto(resolve(href as '/'));
			}
			return;
		}
		filters.domainId = next;

		// The alert list narrows to the domain in place.
		if (pathname.startsWith('/alerts')) {
			syncScopeUrl();
			return;
		}
		if (next !== null) {
			await goto(resolve(`/domains/${next}`));
		} else if (pathname.startsWith('/domains/')) {
			if (filters.organizationId !== null) {
				await goto(resolve(`/organizations/${filters.organizationId}`));
			} else {
				await goto(resolve('/domains'));
			}
		}
		syncScopeUrl();
	}

	function onSinceChange(value: string) {
		filters.since = value;
		syncScopeUrl();
	}
</script>

{#if !hideAll}
	<div class={cn('flex justify-end', className)}>
		<div bind:this={ref} class="flex flex-wrap items-center justify-end gap-2">
			{#if !hideScopeSelectors && !scopeFailed}
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
						{@const currentOrganizationValue = (() => {
							if (pathname.startsWith('/organizations/') && page.params.id) {
								return page.params.id;
							}
							if (pathname.startsWith('/domains/') && page.params.id) {
								const dom = data.domains.find((d) => String(d.id) === page.params.id);
								if (dom) return String(dom.organization_id);
							}
							return filters.organizationId?.toString() ?? 'all';
						})()}

						{@const organizationItems = [
							{ value: 'all', label: 'All organizations' },
							...data.organizations.map((c) => ({ value: String(c.id), label: c.name }))
						]}

						{@const availableDomains =
							currentOrganizationValue !== 'all'
								? data.domains.filter((d) => String(d.organization_id) === currentOrganizationValue)
								: data.domains}

						{@const currentDomainValue = (() => {
							if (pathname.startsWith('/domains/') && page.params.id) {
								return page.params.id;
							}
							return filters.domainId?.toString() ?? 'all';
						})()}

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
							onValueChange={(v) => onOrganizationChange(v, data.domains)}
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
