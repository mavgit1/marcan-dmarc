<script lang="ts">
	import { resolve } from '$app/paths';
	import { page } from '$app/state';
	import LogoMark from '$lib/components/logo-mark.svelte';
	import StatusBadge from '$lib/components/status-badge.svelte';
	import * as Sidebar from '$lib/components/ui/sidebar/index.js';
	import { DOCS_URL, NAV_FOOTER, NAV_GROUPS, isNavActive, type NavItem } from '$lib/nav';
	import { meta } from '$lib/state/meta.svelte';

	const pathname = $derived(page.url.pathname);

	// The free-software notice comes with `meta` (no settings load). The operator's name and logo
	// are for the emails only: the UI carries no operator branding.
	$effect(() => {
		meta.load().catch(() => {});
	});
	/**
	 * Under the mark: the notice line while the server says it is shown (it is unless an operator
	 * key switched it off), else the "Operator license" badge. Under Settings: the version (linking
	 * the release notes), the docs and "0.3.1 available" on the same line when the version check
	 * found a newer one.
	 */
	const current = $derived(meta.current);
	const footLink = 'hover:text-foreground underline-offset-2 hover:underline';
</script>

{#snippet navItem(item: NavItem)}
	{@const active = isNavActive(item.href, pathname)}
	<Sidebar.MenuItem>
		<Sidebar.MenuButton isActive={active} tooltipContent={item.title}>
			{#snippet child({ props })}
				<a href={resolve(item.href)} {...props} aria-current={active ? 'page' : undefined}>
					<item.icon />
					<span>{item.title}</span>
					{#if item.tag}
						<StatusBadge
							tone="warn"
							class="ml-auto h-4 shrink-0 px-1.5 text-[10px] group-data-[collapsible=icon]:hidden"
							>{item.tag}</StatusBadge
						>
					{/if}
				</a>
			{/snippet}
		</Sidebar.MenuButton>
	</Sidebar.MenuItem>
{/snippet}

<Sidebar.Root collapsible="icon">
	<Sidebar.Header>
		<a
			href={resolve('/')}
			class="flex items-center gap-2 rounded-md px-2 py-1.5 outline-none focus-visible:ring-2 focus-visible:ring-sidebar-ring group-data-[collapsible=icon]:justify-center group-data-[collapsible=icon]:px-0"
		>
			<LogoMark class="size-5" />
			<span class="truncate text-sm font-semibold tracking-tight group-data-[collapsible=icon]:hidden">Marcan</span>
		</a>
		{#if current?.notice_shown}
			<p class="text-muted-foreground px-2 text-[11px] leading-tight group-data-[collapsible=icon]:hidden">
				Free software ·
				<a href={current.source_url} target="_blank" rel="noopener" class={footLink}>source</a>
				·
				<a href={current.license_url} target="_blank" rel="noopener" class={footLink}>license</a>
			</p>
		{:else if current?.licensed_to}
			<a
				href={resolve('/settings/license')}
				class="w-fit rounded-md px-2 outline-none focus-visible:ring-2 focus-visible:ring-sidebar-ring group-data-[collapsible=icon]:hidden"
			>
				<StatusBadge tone="neutral" class="h-4 px-1.5 text-[10px]">Operator license</StatusBadge>
			</a>
		{/if}
	</Sidebar.Header>
	<Sidebar.Content>
		{#each NAV_GROUPS as group (group.label)}
			<Sidebar.Group>
				<Sidebar.GroupLabel>{group.label}</Sidebar.GroupLabel>
				<Sidebar.GroupContent>
					<Sidebar.Menu>
						{#each group.items as item (item.href)}
							{@render navItem(item)}
						{/each}
					</Sidebar.Menu>
				</Sidebar.GroupContent>
			</Sidebar.Group>
		{/each}
	</Sidebar.Content>
	<Sidebar.Footer class="border-t border-sidebar-border">
		<Sidebar.Menu>
			{#each NAV_FOOTER as item (item.href)}
				{@render navItem(item)}
			{/each}
		</Sidebar.Menu>
		{#if current}
			<p
				class="text-muted-foreground truncate px-2 text-[11px] leading-tight whitespace-nowrap group-data-[collapsible=icon]:hidden"
			>
				Marcan <a href="{DOCS_URL}releases" target="_blank" rel="noopener" class="tabular-nums {footLink}"
					>{current.version}</a
				>
				·
				<a href={DOCS_URL} target="_blank" rel="noopener" class={footLink}>Docs</a>
				{#if current.available}
					·
					<a href={current.available.url} target="_blank" rel="noopener" class="text-foreground {footLink}"
						><span class="tabular-nums">{current.available.version}</span> available</a
					>
				{/if}
			</p>
		{/if}
	</Sidebar.Footer>
	<Sidebar.Rail />
</Sidebar.Root>
