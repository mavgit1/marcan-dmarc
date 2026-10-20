<script lang="ts">
	import { getClientReportEmail } from '$lib/api';
	import ErrorState from '$lib/components/error-state.svelte';
	import Skeleton from '$lib/components/skeleton.svelte';
	import { Button } from '$lib/components/ui/button/index.js';
	import * as Sheet from '$lib/components/ui/sheet/index.js';
	import type { ClientReportEmail } from '$lib/types/ClientReportEmail';
	import EyeIcon from '@lucide/svelte/icons/eye';

	/**
	 * "Preview": one organization's client report as it would go out now, in a sheet (full
	 * height, full screen on a phone). The header names the subject and the sender; the HTML
	 * shows in a sandboxed frame (no scripts, no links followed), the plain text one click away.
	 * Loaded each time it opens, so it is never older than the page; it sends nothing.
	 */
	let { organizationId, organizationName }: { organizationId: number; organizationName: string } = $props();

	let open = $state(false);
	let email = $state.raw<ClientReportEmail | null>(null);
	let error = $state<unknown>(null);
	let plain = $state(false);

	let seq = 0;
	async function load() {
		const id = ++seq;
		email = null;
		error = null;
		try {
			const loaded = await getClientReportEmail(organizationId);
			if (id === seq) email = loaded;
		} catch (err) {
			if (id === seq) error = err;
		}
	}

	$effect(() => {
		if (open) void load();
	});
</script>

<Button type="button" variant="outline" size="sm" class="gap-1.5" onclick={() => (open = true)}>
	<EyeIcon class="size-3.5" />
	Preview
</Button>

<Sheet.Root bind:open>
	<Sheet.Content side="right" class="gap-0 data-[side=right]:w-full data-[side=right]:sm:max-w-3xl">
		<Sheet.Header class="border-b pr-12">
			<Sheet.Title class="text-sm leading-snug font-semibold break-words">
				{email?.subject ?? `${organizationName}: client report`}
			</Sheet.Title>
			<Sheet.Description class="text-xs">
				{#if email}
					From {email.from_name}. As it would go out now; nothing is sent.
				{:else}
					As it would go out now; nothing is sent.
				{/if}
			</Sheet.Description>
			{#if email}
				<div class="mt-2">
					<Button type="button" variant="ghost" size="sm" class="h-7 px-2 text-xs" onclick={() => (plain = !plain)}>
						{plain ? 'Show HTML' : 'Show plain text'}
					</Button>
				</div>
			{/if}
		</Sheet.Header>
		<div class="flex min-h-0 flex-1 flex-col">
			{#if error}
				<div class="p-4">
					<ErrorState title="The preview failed to load" {error} onRetry={() => void load()} />
				</div>
			{:else if !email}
				<div class="space-y-3 p-4">
					<Skeleton class="h-6 w-2/3" />
					<Skeleton variant="block" class="h-64" />
				</div>
			{:else if plain}
				<pre class="min-h-0 flex-1 overflow-auto p-4 font-mono text-xs whitespace-pre-wrap">{email.text}</pre>
			{:else}
				<iframe
					title="{organizationName}'s client report"
					srcdoc={email.html}
					sandbox=""
					class="min-h-0 w-full flex-1 bg-white"
				></iframe>
			{/if}
		</div>
	</Sheet.Content>
</Sheet.Root>
