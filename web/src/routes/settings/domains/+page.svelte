<script lang="ts">
	import { ListDraft } from '$lib/components/settings/list-draft.svelte';
	import ListEditor from '$lib/components/settings/list-editor.svelte';
	import { DOMAIN_LIST } from '$lib/components/settings/lists';
	import SettingsPage from '$lib/components/settings/settings-page.svelte';
	import { statusText } from '$lib/tokens';

	const domains = new ListDraft(DOMAIN_LIST);
</script>

<SettingsPage
	page="domains"
	hint="Every domain Marcan watches, and the organization it belongs to."
	docs="domains#domains-as-text"
>
	<ListEditor
		list={domains}
		example={'Acme,acme.example\nAcme,acme-old.example,parked\nAcme,acme-spoof.example,ignored'}
		docs="domains#domains-as-text"
	>
		{#snippet format()}
			<li>
				One <code class="font-mono">organization,domain</code> per line; <code class="font-mono">,parked</code>: sends
				no mail. Domains you leave out stay as they are.
			</li>
			<!-- The one rule that deletes data stays in sight. -->
			<li class={statusText.warn}>
				<code class="font-mono">,ignored</code>: not yours. Its reports are deleted.
			</li>
		{/snippet}
	</ListEditor>
</SettingsPage>
