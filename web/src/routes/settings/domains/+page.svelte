<script lang="ts">
	import { ListDraft } from '$lib/components/settings/list-draft.svelte';
	import ListEditor from '$lib/components/settings/list-editor.svelte';
	import { DOMAIN_LIST } from '$lib/components/settings/lists';
	import SettingsPage from '$lib/components/settings/settings-page.svelte';

	const domains = new ListDraft(DOMAIN_LIST);
</script>

<SettingsPage
	page="domains"
	hint="Every domain Marcan watches, and the organization it belongs to."
	docs="domains#domains-as-text"
>
	<ListEditor list={domains} example={'Acme,acme.com\nAcme,acme-old.com,parked\nAcme,acme-spoof.com,ignored'}>
		{#snippet format()}
			<li>One <code class="font-mono">organization,domain</code> per line. A new organization name creates it.</li>
			<li><code class="font-mono">,parked</code>: a domain that sends no mail.</li>
			<li>
				<code class="font-mono">,ignored</code>: not yours; its reports are deleted and new ones dropped. Remove it to
				watch the domain again; past reports are re-imported.
			</li>
			<li>Domains you leave out stay as they are.</li>
			<li>
				Quote a name with a comma: <code class="font-mono">"Acme, Inc"</code>.
				<code class="font-mono">#</code> starts a comment.
			</li>
		{/snippet}
	</ListEditor>
</SettingsPage>
