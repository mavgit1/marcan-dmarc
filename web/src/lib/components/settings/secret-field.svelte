<script lang="ts">
	import { Button } from '$lib/components/ui/button/index.js';
	import { Input } from '$lib/components/ui/input/index.js';
	import { tick } from 'svelte';
	import SecretInput from './secret-input.svelte';

	/**
	 * A stored secret in Settings (the mailbox and SMTP passwords, the operator key). With a stored
	 * value that applies and nothing typed, it is a locked field reading "Stored" (or the server's
	 * `storedLabel`, such as the operator key's hint) with Replace (and Remove, with `onRemove`); Replace opens an empty masked field, focused, with Cancel to
	 * go back. Without one it is the masked field. What is stored never comes back; a typed value
	 * replaces it on save, an empty one keeps it (the request leaves it out), and no hint says so.
	 */
	let {
		value = $bindable(''),
		stored,
		clean,
		id,
		label,
		what = 'password',
		placeholder,
		autocomplete,
		onRemove,
		storedLabel,
		class: className,
		inputClass
	}: {
		value?: string;
		/** A value is stored and applies to what is on screen (same server and login). */
		stored: boolean;
		/** The form has no unsaved changes: a save or discard closes an open Replace. */
		clean: boolean;
		id: string;
		/** The field's accessible name ("Password"). */
		label: string;
		/** What it holds, for the show button's name ("Show the password"). */
		what?: string;
		placeholder?: string;
		autocomplete?: 'current-password' | 'new-password' | 'off';
		/** Shown as Remove beside Replace (the operator key). */
		onRemove?: () => void;
		/** What the locked field reads instead of "Stored": a name for the stored value that does not reveal it. */
		storedLabel?: string | null;
		class?: string;
		inputClass?: string;
	} = $props();

	let replacing = $state(false);
	let wasClean = true;
	$effect(() => {
		if (clean && !wasClean) replacing = false;
		wasClean = clean;
	});

	const locked = $derived(stored && !replacing && value === '');

	async function replace() {
		replacing = true;
		await tick();
		document.getElementById(id)?.focus();
	}

	function cancel() {
		value = '';
		replacing = false;
	}
</script>

{#if locked}
	<div class={['flex min-w-0 flex-wrap items-center gap-2', className]}>
		<Input
			{id}
			value={storedLabel ?? 'Stored'}
			readonly
			aria-label={storedLabel ? `${label}: stored, ${storedLabel}` : `${label}: stored`}
			class={['text-muted-foreground min-w-0 flex-1 text-sm', storedLabel ? 'basis-64' : 'basis-32']}
		/>
		<Button type="button" variant="outline" size="sm" onclick={replace}>Replace</Button>
		{#if onRemove}
			<Button type="button" variant="ghost" size="sm" onclick={onRemove}>Remove</Button>
		{/if}
	</div>
{:else}
	<div class={['flex min-w-0 flex-wrap items-center gap-2', className]}>
		<SecretInput
			{id}
			{what}
			bind:value
			{placeholder}
			{autocomplete}
			aria-label={label}
			class="min-w-0 flex-1 basis-32"
			{inputClass}
		/>
		{#if stored}
			<Button type="button" variant="ghost" size="sm" onclick={cancel}>Cancel</Button>
		{/if}
	</div>
{/if}
