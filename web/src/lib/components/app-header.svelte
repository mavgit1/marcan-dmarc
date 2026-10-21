<script lang="ts">
	import { tick } from 'svelte';
	import { resolve } from '$app/paths';
	import { page } from '$app/state';
	import CircleArrowUpIcon from '@lucide/svelte/icons/circle-arrow-up';
	import TriangleAlertIcon from '@lucide/svelte/icons/triangle-alert';
	import ImportProgress from '$lib/components/import-progress.svelte';
	import ModeToggle from '$lib/components/mode-toggle.svelte';
	import ScopeFilters from '$lib/components/scope-filters.svelte';
	import * as Breadcrumb from '$lib/components/ui/breadcrumb/index.js';
	import { Separator } from '$lib/components/ui/separator/index.js';
	import * as Sidebar from '$lib/components/ui/sidebar/index.js';
	import * as Popover from '$lib/components/ui/popover/index.js';
	import MailboxSync from '$lib/components/settings/mailbox-sync.svelte';
	import StatusBadge from '$lib/components/status-badge.svelte';
	import { formatDateTime, formatInt, formatTimeOfDay, plural } from '$lib/format';
	import { crumbsForPath, titleForPath, type CrumbOrganization } from '$lib/nav';
	import { filters } from '$lib/state/filters.svelte';
	import { meta } from '$lib/state/meta.svelte';
	import { scopeOptions } from '$lib/state/scope.svelte';
	import { pageMeta } from '$lib/state/page-meta.svelte';
	import { progressPercent } from '$lib/imap-progress';
	import { pipelineStatus } from '$lib/pipeline';
	import { refresh } from '$lib/state/refresh.svelte';
	import { systemHealth } from '$lib/state/system-health.svelte';
	import { statusDot } from '$lib/tokens';
	import { cn } from '$lib/utils.js';

	const pathname = $derived(page.url.pathname);
	const title = $derived(titleForPath(pathname));
	/**
	 * The organization the scope is narrowed to (in the scope filter, or by a link from its page): a
	 * domain or sender page's trail starts there. A domain page names the domain's own
	 * organization, like the scope filter beside it.
	 */
	const crumbOrganization = $derived.by((): CrumbOrganization | null => {
		if (filters.organizationId === null) return null;
		const scope = scopeOptions.current;
		let id = filters.organizationId;
		if (pathname.startsWith('/domains/')) {
			const domain = scope?.domains.find((d) => String(d.id) === page.params.id);
			if (domain) id = domain.organization_id;
		}
		const name = scope?.organizations.find((o) => o.id === id)?.name;
		return { id, name: name ?? `Organization ${id}` };
	});
	const crumbs = $derived(crumbsForPath(pathname, page.params, pageMeta.entityName, crumbOrganization));
	const updatedLabel = $derived(refresh.lastUpdatedAt == null ? null : formatTimeOfDay(refresh.lastUpdatedAt));

	// The header is always mounted, so it keeps the shared health poll running
	// (30 s, or 3 s while the mailbox is being imported).
	$effect(() => systemHealth.track());
	$effect(() => {
		void refresh.tick;
		void systemHealth.poll();
	});

	const health = $derived(systemHealth.current);
	const importing = $derived(health?.imap_progress ?? null);
	/** Same verdicts as Settings: one definition in `$lib/pipeline`. */
	const status = $derived(health ? pipelineStatus(health) : null);
	const importPct = $derived(importing ? progressPercent(importing) : null);

	const headerButtonLabel = $derived.by(() => {
		if (importing?.phase === 'importing') {
			return importPct != null ? `Importing ${Math.floor(importPct)}%` : 'Importing';
		}
		return status?.overall.label ?? '';
	});

	const lastIngestFormatted = $derived(
		health?.last_ingest_at == null ? 'No reports yet' : formatDateTime(health.last_ingest_at)
	);

	/** A newer version the version check found (the sidebar's foot links it too). */
	const available = $derived(meta.current?.available ?? null);
	$effect(() => {
		meta.load().catch(() => {});
	});

	const skippedCount = $derived(health?.quarantine_count ?? 0);
	const skippedLabel = $derived(plural(skippedCount, 'skipped report'));

	/**
	 * The title row always holds the trigger, the breadcrumb, the status pill (with the update
	 * pill before it while a newer version is out) and the theme toggle. The filters sit between the breadcrumb and the status while all of them fit at
	 * their natural width, otherwise on a row of their own below, right-aligned. CSS cannot say
	 * this: a wrapping flex row moves its trailing items (the status) along with the filters,
	 * and a breakpoint cannot know how wide the selected names are. So the header measures.
	 * Where even the title row alone does not fit at its natural width (a phone), the update
	 * pill drops its word and shows an arrow instead ("↑ 0.3.1"), so the breadcrumb keeps room.
	 */
	let headerEl = $state<HTMLElement | null>(null);
	let rulerEl = $state<HTMLElement | null>(null);
	let crumbEl = $state<HTMLElement | null>(null);
	let statusEl = $state<HTMLElement | null>(null);
	let filtersEl = $state<HTMLElement | null>(null);
	let filtersBelow = $state(false);
	let updateCompact = $state(false);

	/**
	 * An element's width on one line (max-content), from an invisible copy; the update pill in
	 * the copy always at its full label, so its own shortening never feeds back.
	 */
	function naturalWidth(el: HTMLElement, ruler: HTMLElement): number {
		const copy = el.cloneNode(true) as HTMLElement;
		copy.style.cssText =
			'position:absolute;top:0;left:0;visibility:hidden;width:max-content;max-width:none;min-width:0';
		for (const word of copy.querySelectorAll('[data-update-word]')) word.classList.remove('sr-only');
		for (const icon of copy.querySelectorAll('[data-update-icon]')) icon.remove();
		ruler.append(copy);
		const width = copy.getBoundingClientRect().width;
		copy.remove();
		return width;
	}

	function placeFilters() {
		if (!headerEl || !rulerEl || !crumbEl) return;
		const filters = filtersEl;
		const gap = parseFloat(getComputedStyle(headerEl).columnGap) || 0;
		const rowWidth = rulerEl.getBoundingClientRect().width;
		// The title row without the filters, at its natural width with the full update pill.
		let titleRow = 0;
		for (const child of headerEl.children) {
			const el = child as HTMLElement;
			if (el === rulerEl || (filters && el.contains(filters)) || el.getClientRects().length === 0) continue;
			const width = el === crumbEl || el === statusEl ? naturalWidth(el, rulerEl) : el.getBoundingClientRect().width;
			titleRow += (titleRow > 0 ? gap : 0) + width;
		}
		updateCompact = titleRow > rowWidth + 0.5;
		if (filters) {
			// Everything on the title row at its natural width: the filters and each shown item.
			let needed = naturalWidth(filters, rulerEl);
			for (const child of headerEl.children) {
				const el = child as HTMLElement;
				if (el === rulerEl || el.contains(filters) || el.getClientRects().length === 0) continue;
				const width = el === crumbEl ? naturalWidth(el, rulerEl) : el.getBoundingClientRect().width;
				needed += gap + width;
			}
			filtersBelow = needed > rowWidth + 0.5;
		} else {
			filtersBelow = false;
		}
		void tick().then(publishHeight);
	}

	/**
	 * The header's height as `--app-header-height` on the root, so what sticks below it (a
	 * table's column header) sits under it however many lines the header wraps to. Written after
	 * each placement rather than by an observer on the header: placing the filters changes the
	 * header's height, and an observer whose target resizes inside its own callback loops.
	 */
	function publishHeight() {
		if (!headerEl) return;
		document.documentElement.style.setProperty('--app-header-height', `${headerEl.offsetHeight}px`);
	}

	// The row's width and the status's and filters' labels can each move the filters. Moving
	// them resizes none of these; shortening the update pill resizes the status once, and the
	// next placement measures the pill at its full label again, so it settles.
	$effect(() => {
		const targets = [rulerEl, statusEl, filtersEl].filter((el) => el !== null);
		const observer = new ResizeObserver(() => placeFilters());
		for (const el of targets) observer.observe(el);
		return () => observer.disconnect();
	});
	// The breadcrumb follows the page (and the entity name once it loads).
	$effect(() => {
		void crumbs;
		void title;
		placeFilters();
	});
	// The webfont changes every width once it is in.
	$effect(() => {
		void document.fonts?.ready.then(() => placeFilters());
	});
</script>

<header
	bind:this={headerEl}
	class="bg-background/80 sticky top-0 z-20 flex min-h-12 flex-wrap items-center gap-2 border-b px-3 py-2 backdrop-blur-sm"
>
	<!-- The row's inner width, unchanged when the filters move (measuring copies go in here). -->
	<div
		bind:this={rulerEl}
		aria-hidden="true"
		class="pointer-events-none invisible absolute inset-x-3 top-0 h-0 overflow-visible"
	></div>
	<Sidebar.Trigger class="shrink-0" />
	<Separator orientation="vertical" class="hidden shrink-0 data-[orientation=vertical]:h-4 sm:block" />
	<div bind:this={crumbEl} class="min-w-0 flex-1">
		{#if crumbs.length > 1}
			<Breadcrumb.Root>
				<Breadcrumb.List>
					{#each crumbs as crumb, i (i)}
						<Breadcrumb.Item>
							{#if crumb.href && i < crumbs.length - 1}
								<Breadcrumb.Link href={resolve(crumb.href as '/')}>{crumb.label}</Breadcrumb.Link>
							{:else}
								<Breadcrumb.Page>{crumb.label}</Breadcrumb.Page>
							{/if}
						</Breadcrumb.Item>
						{#if i < crumbs.length - 1}
							<Breadcrumb.Separator />
						{/if}
					{/each}
				</Breadcrumb.List>
			</Breadcrumb.Root>
		{:else}
			<h1 class="truncate text-sm font-medium">{title}</h1>
		{/if}
	</div>
	<ScopeFilters bind:ref={filtersEl} class={filtersBelow ? 'order-last basis-full' : undefined} />
	<div bind:this={statusEl} class="ml-auto flex shrink-0 items-center gap-1.5">
		{#if available}
			<a
				href={available.url}
				target="_blank"
				rel="noopener"
				class="focus-visible:ring-ring rounded-md focus-visible:ring-1 focus-visible:outline-none"
			>
				<StatusBadge tone="neutral" class="hover:text-foreground h-5 gap-1 px-1.5 text-[11px] font-medium">
					{#if updateCompact}
						<span data-update-icon class="inline-flex"><CircleArrowUpIcon class="size-3 shrink-0" /></span>
					{/if}
					<span class="tabular-nums">{available.version}</span>
					<span data-update-word class={updateCompact ? 'sr-only' : undefined}>available</span>
				</StatusBadge>
			</a>
		{/if}
		{#if health}
			<Popover.Root>
				<Popover.Trigger>
					{#snippet child({ props })}
						<button
							{...props}
							type="button"
							class="hover:bg-muted/80 hover:text-foreground focus-visible:ring-ring inline-flex cursor-pointer items-center gap-2 rounded-md px-2 py-1 text-xs text-muted-foreground transition-colors focus-visible:outline-none focus-visible:ring-1"
							aria-label="System status"
							title={updatedLabel ? `Updated ${updatedLabel}` : undefined}
						>
							<span class="inline-flex items-center gap-1.5 font-medium">
								<span
									class={cn('size-2 shrink-0 rounded-full', statusDot[status?.overall.tone ?? 'neutral'])}
									aria-hidden="true"
								></span>
								<span>{headerButtonLabel}</span>
							</span>
							{#if skippedCount > 0}
								<StatusBadge tone="warn" class="h-5 gap-1 px-1.5 text-[11px] font-medium">
									<TriangleAlertIcon class="size-3 shrink-0" />
									<span>{skippedLabel}</span>
								</StatusBadge>
							{/if}
						</button>
					{/snippet}
				</Popover.Trigger>
				<Popover.Content side="bottom" align="end" class="w-80">
					<div class="space-y-2 text-xs">
						<div class="border-b pb-1.5 flex items-center justify-between font-medium">
							<span class="text-foreground">System health</span>
							<span class="text-muted-foreground flex items-center gap-1.5 font-normal">
								<span
									class={cn('size-1.5 shrink-0 rounded-full', statusDot[status?.overall.tone ?? 'neutral'])}
									aria-hidden="true"
								></span>
								{headerButtonLabel}
							</span>
						</div>

						<div class="grid grid-cols-[auto_1fr] gap-x-4 gap-y-1.5 pt-0.5">
							<span class="text-muted-foreground flex items-center gap-1.5">
								<span
									class={cn('size-1.5 shrink-0 rounded-full', statusDot[status?.mailbox.tone ?? 'neutral'])}
									aria-hidden="true"
								></span>
								Mailbox check
							</span>
							<span class="text-foreground text-right font-medium tabular-nums">
								{status?.mailbox.label}{status?.mailbox.when && !importing ? ` · ${status.mailbox.when}` : ''}
							</span>
							{#if importing}
								<ImportProgress progress={importing} class="col-span-2" />
							{:else if health.imap_last_error}
								<p class="text-status-fail-fg col-span-2 break-words text-[11px]">
									{health.imap_last_error}
								</p>
							{/if}

							<span class="text-muted-foreground flex items-center gap-1.5">
								<span
									class={cn('size-1.5 shrink-0 rounded-full', statusDot[status?.reports.tone ?? 'neutral'])}
									aria-hidden="true"
								></span>
								Latest report
							</span>
							<span class="text-foreground text-right font-medium tabular-nums" title={lastIngestFormatted}>
								{status?.reports.label}{status?.reports.when ? ` · ${status.reports.when}` : ''}
							</span>

							<span class="text-muted-foreground">Skipped reports</span>
							<span
								class={cn(
									'text-right font-medium tabular-nums',
									skippedCount > 0 ? 'text-status-warn-fg' : 'text-muted-foreground'
								)}
							>
								{formatInt(skippedCount)}
							</span>
						</div>
						<div class="flex items-center justify-between gap-2 border-t pt-2.5">
							<a
								href={resolve('/settings/mailbox')}
								class="text-muted-foreground hover:text-foreground underline-offset-2 hover:underline"
							>
								Mailbox settings
							</a>
							<MailboxSync {health} variant="button" />
						</div>
					</div>
				</Popover.Content>
			</Popover.Root>
		{/if}
		<ModeToggle />
	</div>
</header>
