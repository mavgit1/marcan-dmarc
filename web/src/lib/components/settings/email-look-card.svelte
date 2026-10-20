<script lang="ts">
	import LogoMark from '$lib/components/logo-mark.svelte';
	import type { EmailLook } from '$lib/types/EmailLook';

	/**
	 * One email as a client receives it, small: "From:" and the sender name, the top (Marcan's
	 * line, the operator's letterhead, or both), the title, two bars for the body and the last
	 * line, all as the server answered (`EmailLook`, the code the emails use). The Branding page's
	 * "How your emails look" and the License page's "What your clients see".
	 */
	let { look, label, logoUrl }: { look: EmailLook; label: string; logoUrl: string | null } = $props();
</script>

<div class="bg-background max-w-xl rounded-lg border p-4" aria-label={label} role="figure">
	<p class="text-muted-foreground mb-3 text-xs break-words">From: {look.from_name}</p>
	{#if look.top.software}
		<p class="text-muted-foreground mb-3 flex items-center gap-1.5 text-[13px]">
			<span
				class="inline-flex shrink-0"
				style:width="{look.top.software.mark.width}px"
				style:height="{look.top.software.mark.height}px"
			>
				<LogoMark class="size-full" />
			</span>
			<span><span class="font-semibold">{look.top.software.wordmark}</span>{look.top.software.rest}</span>
		</p>
	{/if}
	{#if look.top.operator}
		{@const operator = look.top.operator}
		{#if operator.prominent}
			{#if operator.logo && logoUrl}
				<img
					src={logoUrl}
					alt={operator.name ?? ''}
					width={operator.logo.width}
					height={operator.logo.height}
					class="mb-2 block max-w-full"
				/>
			{/if}
			{#if operator.name}<p class="text-xl font-bold break-words">{operator.name}</p>{/if}
		{:else}
			<p class="text-foreground/75 mb-3 flex min-w-0 items-center gap-1.5 text-[13px]">
				{#if operator.logo && logoUrl}
					<img src={logoUrl} alt="" width={operator.logo.width} height={operator.logo.height} class="shrink-0" />
				{/if}
				<span class="break-words">{operator.name}</span>
			</p>
		{/if}
	{/if}
	<p class="text-base font-semibold break-words">{look.title}</p>
	<div class="my-4 space-y-2" aria-hidden="true">
		<div class="bg-muted h-2 w-3/4 rounded-full"></div>
		<div class="bg-muted h-2 w-1/2 rounded-full"></div>
	</div>
	<p class="text-muted-foreground text-xs break-words">{look.sign_off}</p>
</div>
