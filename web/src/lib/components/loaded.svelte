<script lang="ts" generics="T">
	import ErrorState from '$lib/components/error-state.svelte';
	import RefreshError from '$lib/components/refresh-error.svelte';
	import { refresh } from '$lib/state/refresh.svelte';
	import type { Resource } from '$lib/state/resource.svelte';
	import type { Snippet } from 'svelte';

	/**
	 * The one shell around a `resource()`: its data (with `RefreshError` above it when the
	 * latest refresh failed, the last good data stays), else the error (`ErrorState` with a
	 * retry, or the `error` snippet), else the `loading` skeleton. One block per state, so
	 * a refresh re-renders the data and never remounts it.
	 */
	let {
		source,
		errorTitle,
		header,
		data,
		loading,
		error
	}: {
		source: Resource<T>;
		/** Title of the default `ErrorState`. */
		errorTitle: string;
		/** Rendered above the refresh error line (a page header that belongs to the data). */
		header?: Snippet<[T]>;
		data: Snippet<[T]>;
		loading: Snippet;
		/** Replaces the default `ErrorState` (e.g. to keep a page header above it). */
		error?: Snippet<[unknown]>;
	} = $props();
</script>

{#if source.data !== null}
	{@render header?.(source.data)}
	{#if source.error}
		<RefreshError error={source.error} />
	{/if}
	{@render data(source.data)}
{:else if source.error}
	{#if error}
		{@render error(source.error)}
	{:else}
		<ErrorState title={errorTitle} error={source.error} onRetry={() => refresh.refresh()} />
	{/if}
{:else}
	{@render loading()}
{/if}
