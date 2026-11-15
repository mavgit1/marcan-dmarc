<script lang="ts">
	import { SECURITY_LABEL, type Security } from './connection';
	import { SELECT_CLASS } from './select-class';

	/**
	 * A server's TLS / STARTTLS choice next to its port (an account's IMAP and SMTP servers):
	 * `onchange` gets the pick, and the account model moves a port that is empty or still holds the
	 * other choice's usual number to its own, keeping a typed one (`portAfterSecurityChange`).
	 */
	let {
		value,
		onchange,
		id,
		disabled = false,
		class: className
	}: {
		value: Security;
		onchange: (security: Security) => void;
		id: string;
		disabled?: boolean;
		class?: string;
	} = $props();

	const CHOICES: Security[] = ['tls', 'starttls'];
</script>

<select
	{id}
	{disabled}
	{value}
	onchange={(e) => onchange(e.currentTarget.value as Security)}
	class={[SELECT_CLASS, 'h-9 w-full min-w-0', className]}
>
	{#each CHOICES as choice (choice)}
		<option value={choice}>{SECURITY_LABEL[choice]}</option>
	{/each}
</select>
