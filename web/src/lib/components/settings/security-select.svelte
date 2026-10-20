<script lang="ts">
	import { SECURITY_LABEL, type Security } from './connection';

	/**
	 * An account's TLS / STARTTLS choice next to its port (the mailbox in both mailbox forms, the
	 * sending account): picking one moves a port that is empty or still holds the other choice's
	 * usual number to its own, and keeps a typed one (the model's `setSecurity`, over
	 * `portAfterSecurityChange`).
	 */
	let {
		model,
		id,
		disabled = false,
		class: className
	}: {
		model: { readonly saved: unknown; readonly draft: { security: Security }; setSecurity(s: Security): void };
		id: string;
		disabled?: boolean;
		class?: string;
	} = $props();

	const CHOICES: Security[] = ['tls', 'starttls'];
</script>

{#if model.saved}
	<select
		{id}
		{disabled}
		value={model.draft.security}
		onchange={(e) => model.setSecurity(e.currentTarget.value as Security)}
		class={[
			'border-input bg-background h-9 w-full min-w-0 rounded-md border px-2 text-sm shadow-xs disabled:opacity-50',
			className
		]}
	>
		{#each CHOICES as choice (choice)}
			<option value={choice}>{SECURITY_LABEL[choice]}</option>
		{/each}
	</select>
{/if}
