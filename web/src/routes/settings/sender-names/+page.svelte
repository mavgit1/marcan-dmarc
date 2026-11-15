<script lang="ts">
	import { ListDraft } from '$lib/components/settings/list-draft.svelte';
	import ListEditor from '$lib/components/settings/list-editor.svelte';
	import { SENDER_NAME_LIST } from '$lib/components/settings/lists';
	import SettingsPage from '$lib/components/settings/settings-page.svelte';

	const names = new ListDraft(SENDER_NAME_LIST);
</script>

<SettingsPage
	page="sender-names"
	hint="Your own names for senders, in place of the built-in ones."
	docs="sender-names#your-own-names"
>
	<ListEditor
		list={names}
		example={'crm.acme.example,Our CRM\n192.0.2.10,Office printer\n192.0.2.0/24,Office\nmailer.example,'}
		docs="sender-names#your-own-names"
	>
		{#snippet format()}
			<li>
				One <code class="font-mono">target,name</code> per line: a sender's domain or a host under it, an IP, or an IP range.
			</li>
			<li>An empty name hides the built-in one.</li>
		{/snippet}
	</ListEditor>
</SettingsPage>
