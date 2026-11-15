<script lang="ts">
	import { page } from '$app/state';
	import AccountPage from '$lib/components/settings/account-page.svelte';

	/** A non-numeric id is an account that does not exist (-1 is never one). */
	const id = $derived(/^\d+$/.test(page.params.id ?? '') ? Number(page.params.id) : -1);
</script>

{#key id}
	<AccountPage {id} />
{/key}
