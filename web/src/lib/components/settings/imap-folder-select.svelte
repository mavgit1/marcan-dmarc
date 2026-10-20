<script lang="ts">
	import type { HTMLSelectAttributes } from 'svelte/elements';
	import type { ImapModel } from './imap-draft.svelte';

	/**
	 * The folder to import, from the folders the last login listed; the draft's folder stays
	 * listed first when the server did not list it (not tested yet, or gone). `onpick` gets the
	 * picked name: the caller decides what a pick does (set it, test it).
	 */
	let {
		model: m,
		onpick,
		class: className,
		...rest
	}: {
		model: ImapModel;
		onpick: (name: string) => void;
		class?: string;
	} & Omit<HTMLSelectAttributes, 'value' | 'onchange' | 'class'> = $props();

	const folder = $derived(m.draft.folder);
</script>

<select
	{...rest}
	value={folder}
	onchange={(e) => onpick(e.currentTarget.value)}
	class={['border-input bg-background h-9 w-full rounded-md border px-2 text-sm shadow-xs', className]}
>
	{#if !m.folders?.some((f) => f.name === folder)}
		<option value={folder}>{folder}</option>
	{/if}
	{#each m.folders ?? [] as f (f.name)}
		<option value={f.name}>{f.name}</option>
	{/each}
</select>
