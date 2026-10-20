<script lang="ts" generics="T">
	import ErrorState from '$lib/components/error-state.svelte';
	import Skeleton from '$lib/components/skeleton.svelte';
	import type { Snippet } from 'svelte';

	/**
	 * The load shell of one Settings part (a group's form, a list, a row), like `Loaded` for a
	 * page: the part (`children`, handed the saved value) once its `SettingsForm` has loaded,
	 * else `ErrorState` with a retry when the first load failed, else a block skeleton of the
	 * part's size (`skeleton`, classes). Later failures keep the part on screen.
	 */
	let {
		form,
		errorTitle,
		skeleton = 'h-20',
		children
	}: {
		form: { readonly loadError: unknown; readonly saved: T | null; reload(): void };
		/** "Mailbox settings failed to load". */
		errorTitle: string;
		skeleton?: string;
		children: Snippet<[T]>;
	} = $props();
</script>

{#if form.saved !== null}
	{@render children(form.saved)}
{:else if form.loadError}
	<ErrorState title={errorTitle} error={form.loadError} onRetry={() => form.reload()} />
{:else}
	<Skeleton variant="block" class={skeleton} />
{/if}
