<script lang="ts">
	import EyeIcon from '@lucide/svelte/icons/eye';
	import EyeOffIcon from '@lucide/svelte/icons/eye-off';
	import { Input } from '$lib/components/ui/input/index.js';
	import type { HTMLInputAttributes } from 'svelte/elements';

	/**
	 * A secret typed into Settings (a password, the operator key): masked, with a show / hide
	 * button inside the field. What is stored never comes back, so the field starts empty; once
	 * it is empty again (saved or discarded) it is masked again. `what` words the button
	 * ("Show the password"). `class` places the field; the rest goes to the input.
	 */
	let {
		value = $bindable(''),
		what = 'password',
		class: className,
		inputClass,
		...rest
	}: Omit<HTMLInputAttributes, 'type' | 'value' | 'class' | 'files'> & {
		value?: string;
		/** What the field holds, for the button's name. */
		what?: string;
		class?: string;
		inputClass?: string;
	} = $props();

	let shown = $state(false);
	$effect(() => {
		if (!value) shown = false;
	});
</script>

<div class={['relative', className]}>
	<Input
		{...rest}
		type={shown ? 'text' : 'password'}
		bind:value
		spellcheck={false}
		class="pr-9 text-sm {inputClass ?? ''}"
	/>
	<button
		type="button"
		class="text-muted-foreground hover:text-foreground focus-visible:ring-ring/50 absolute inset-y-0 right-0 flex w-9 items-center justify-center rounded-r-lg outline-none focus-visible:ring-3 disabled:pointer-events-none disabled:opacity-50"
		aria-label="{shown ? 'Hide' : 'Show'} the {what}"
		disabled={rest.disabled}
		onclick={() => (shown = !shown)}
	>
		{#if shown}<EyeOffIcon class="size-4" />{:else}<EyeIcon class="size-4" />{/if}
	</button>
</div>
