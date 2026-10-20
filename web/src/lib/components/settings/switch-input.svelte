<script lang="ts">
	/**
	 * An on / off switch: a native checkbox (`role="switch"`) drawn as a track and a knob, so a
	 * `<label for={id}>` names it and a form reads it like any checkbox. The control of a feature
	 * page's switch row (`SettingsPage`).
	 */
	let {
		id,
		checked,
		onchange
	}: {
		id: string;
		checked: boolean;
		onchange: (checked: boolean) => void;
	} = $props();
</script>

<span class="relative inline-flex h-5 w-9 shrink-0">
	<input
		{id}
		type="checkbox"
		role="switch"
		{checked}
		onchange={(e) => onchange(e.currentTarget.checked)}
		class="peer absolute inset-0 z-10 m-0 size-full cursor-pointer opacity-0"
	/>
	<span
		aria-hidden="true"
		class={[
			'peer-focus-visible:ring-ring/50 inline-flex h-5 w-9 items-center rounded-full p-0.5 transition-colors peer-focus-visible:ring-3',
			checked ? 'bg-primary' : 'bg-input dark:bg-input/80'
		]}
	>
		<span
			class={[
				'bg-background block size-4 rounded-full shadow-sm transition-transform',
				checked ? 'translate-x-4' : 'translate-x-0'
			]}
		></span>
	</span>
</span>
