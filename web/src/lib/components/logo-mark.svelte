<script lang="ts">
	import { cn } from '$lib/utils.js';

	let { class: className }: { class?: string } = $props();
</script>

<!--
	"Aggregate" mark: five report bars on one baseline, stepped down to the middle so the silhouette
	reads as an M. Drawn on the 20px grid it renders at (size-5): 3px bars, 1px gaps, every edge on a
	whole pixel at 1x and 2x. It spans x 0–19 so its left edge lines up with the nav icons below.
	Neutral bars follow currentColor; the middle bar uses the status-info blue (light/dark aware).
	Favicons ($lib/assets/favicon*.svg) use a three-bar version that fits the 16px grid.
-->
<svg
	class={cn('size-5 shrink-0', className)}
	viewBox="0 0 20 20"
	fill="currentColor"
	shape-rendering="crispEdges"
	xmlns="http://www.w3.org/2000/svg"
	aria-hidden="true"
>
	<rect x="0" y="2" width="3" height="16" />
	<rect x="4" y="5" width="3" height="13" />
	<rect x="8" y="10" width="3" height="8" class="fill-status-info" />
	<rect x="12" y="5" width="3" height="13" />
	<rect x="16" y="2" width="3" height="16" />
</svg>
