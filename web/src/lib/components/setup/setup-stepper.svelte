<script lang="ts">
	import { page } from '$app/state';
	import { getOrganizations, getScope } from '$lib/api';
	import type { OrganizationSummary } from '$lib/types/OrganizationSummary';
	import Loaded from '$lib/components/loaded.svelte';
	import Skeleton from '$lib/components/skeleton.svelte';
	import { ReportingSetupState } from '$lib/components/reporting-setup/reporting-setup-state.svelte';
	import * as Card from '$lib/components/ui/card/index.js';
	import { resource, type Resource } from '$lib/state/resource.svelte';
	import { settings } from '$lib/state/settings.svelte';
	import { systemHealth } from '$lib/state/system-health.svelte';
	import type { Snippet } from 'svelte';
	import { firstOpenStep, SETUP_STEPS, type SetupData } from './setup-steps';
	import StepDelivery from './step-delivery.svelte';
	import StepDmarc from './step-dmarc.svelte';
	import StepDomains from './step-domains.svelte';
	import StepDone from './step-done.svelte';
	import StepImport from './step-import.svelte';
	import StepList from './step-list.svelte';
	import StepMailbox from './step-mailbox.svelte';
	import StepOrganization from './step-organization.svelte';

	/**
	 * The setup, one step at a time in a fixed order, with Back and Continue. Each step is done
	 * by what the server stored and answers (`SETUP_STEPS`); it opens on the first step that is
	 * not, or on the one a link names (`?step=dmarc`, from the guide). A step can be opened from
	 * the list once reached.
	 */
	let {
		finishing = false,
		header,
		onFinish
	}: {
		/** Finish setup is saving. */
		finishing?: boolean;
		/** Beside the title (Skip setup). */
		header?: Snippet;
		onFinish?: () => void;
	} = $props();

	/** The default organization, with the settings and the first health poll. */
	const setup = resource({
		key: () => 'setup',
		load: (): Promise<OrganizationSummary> =>
			Promise.all([getOrganizations(), settings.load(), systemHealth.current ? null : systemHealth.poll()]).then(
				([organizations]) => {
					const organization = organizations.items.find((c) => c.is_default) ?? organizations.items[0];
					if (!organization) throw new Error('No default organization found.');
					return organization;
				}
			)
	});

	/**
	 * The monitored domains, names only (`GET /api/scope`, no report figures): the steps need
	 * the names and a count, so this alone follows a running import.
	 */
	const domainList = resource({
		key: () => 'setup-domains',
		load: () => getScope().then((scope) => scope.domains),
		reloadOn: () => systemHealth.importTick
	});

	/**
	 * Each domain's reporting status, decided by the server from live DNS lookups: asked when
	 * the setup opens, when the DMARC step is entered and on Re-check (and when an import ends,
	 * with every page), never on the import's ticks.
	 */
	const reporting = new ReportingSetupState({
		enabled: () => !!settings.current?.reporting.report_address,
		reloadOn: () => [settings.current?.reporting.report_address, settings.current?.reporting.reporting_mode]
	});

	/** What the card loads: the organization, unless the domain list failed with nothing shown. */
	const loadSource = $derived<Resource<unknown>>(domainList.error && !domainList.data ? domainList : setup);

	let organizationConfirmed = $state(false);

	/** Everything the steps read: the shared settings and health poll are live. */
	const data = $derived<SetupData | null>(
		setup.data && domainList.data && settings.current && systemHealth.current
			? {
					organization: setup.data,
					domains: domainList.data,
					settings: settings.current,
					health: systemHealth.current,
					reporting: reporting.data,
					organizationConfirmed
				}
			: null
	);
	const done = $derived(data ? SETUP_STEPS.map((s) => s.done(data)) : SETUP_STEPS.map(() => false));

	/** The step on screen, and the furthest reached in this visit; set once the data is in. */
	let current = $state<number | null>(null);
	let reached = $state(0);
	/** The first reporting setup answer is still out: the DMARC step's state is not known yet. */
	const reportingPending = $derived(
		!!settings.current?.reporting.report_address && reporting.data === null && !reporting.source.error
	);
	/** The step a link names (`?step=`); -1 for none. */
	const linkedStep = SETUP_STEPS.findIndex((s) => s.id === page.url.searchParams.get('step'));
	$effect(() => {
		if (data && current === null && !reportingPending) {
			const open = firstOpenStep(data);
			current = linkedStep === -1 ? open : linkedStep;
			reached = Math.max(open, current);
		}
	});

	function go(index: number) {
		const to = Math.max(0, Math.min(SETUP_STEPS.length - 1, index));
		// Entering the DMARC step looks its domains up again (once, not on every import tick).
		if (to !== current && SETUP_STEPS[to].id === 'dmarc') reporting.source.reload();
		current = to;
		reached = Math.max(reached, current);
		if (typeof window !== 'undefined') window.scrollTo({ top: 0 });
	}
	const next = () => go((current ?? 0) + 1);
	const back = () => go((current ?? 0) - 1);
	const reachable = (i: number) => i <= reached;
</script>

<Card.Root class="mx-auto w-full max-w-3xl">
	<Card.Header class="space-y-4 border-b pb-4">
		<div class="flex flex-wrap items-start justify-between gap-3">
			<div class="space-y-1">
				<Card.Title class="text-lg font-semibold">Set up Marcan</Card.Title>
				<Card.Description>From the report mailbox to your first DMARC report, one step at a time.</Card.Description>
			</div>
			{@render header?.()}
		</div>
		{#if current !== null}
			<StepList {current} {done} {reachable} onSelect={go} />
		{:else}
			<Skeleton class="h-12 w-full" />
		{/if}
	</Card.Header>
	<Card.Content class="pt-4">
		<Loaded source={loadSource} errorTitle="Setup failed to load" data={body} loading={skeleton} />
	</Card.Content>
</Card.Root>

{#snippet skeleton()}
	<div class="space-y-3">
		<Skeleton class="h-5 w-48" />
		<Skeleton class="h-24 w-full" />
	</div>
{/snippet}

{#snippet body()}
	{#if !data || current === null}
		<!-- The settings or the first health poll are still out. -->
		{@render skeleton()}
	{:else}
		{@const step = SETUP_STEPS[current]}
		<section class="space-y-4" aria-labelledby="setup-step-heading">
			<h2 id="setup-step-heading" class="text-sm font-medium">{step.heading}</h2>
			{#key step.id}
				{#if step.id === 'organization'}
					<StepOrganization
						organization={data.organization}
						onSaved={(organization) => {
							setup.update(() => organization);
							organizationConfirmed = true;
							next();
						}}
					/>
				{:else if step.id === 'mailbox'}
					<StepMailbox {data} done={done[current]} onBack={back} onNext={next} />
				{:else if step.id === 'import'}
					<StepImport {data} onBack={back} onNext={next} />
				{:else if step.id === 'domains'}
					<StepDomains {data} onChanged={() => domainList.reload()} onBack={back} onNext={next} />
				{:else if step.id === 'delivery'}
					<StepDelivery {data} onBack={back} onNext={next} />
				{:else if step.id === 'dmarc'}
					<StepDmarc {data} {reporting} onBack={back} onNext={next} />
				{:else}
					<StepDone {data} {finishing} onBack={back} {onFinish} />
				{/if}
			{/key}
		</section>
	{/if}
{/snippet}
