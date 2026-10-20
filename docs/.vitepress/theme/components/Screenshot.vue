<script setup lang="ts">
// A screenshot of the app in the docs' current theme. Files live in
// public/screenshots/<name>-light.png and <name>-dark.png, made from the demo
// data (cargo run -p dmarc-store --example demo-data).
import { withBase } from 'vitepress'

defineProps<{ name: string; alt: string }>()
</script>

<template>
  <figure class="screenshot">
    <img class="light-only" :src="withBase(`/screenshots/${name}-light.png`)" :alt="alt" loading="lazy" />
    <img class="dark-only" :src="withBase(`/screenshots/${name}-dark.png`)" :alt="alt" loading="lazy" />
  </figure>
</template>
