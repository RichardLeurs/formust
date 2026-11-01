<script setup>
import { features } from '../data/content.js'
</script>

<template>
  <section id="features" class="features">
    <article v-for="f in features" :key="f.title" class="card">
      <h3>{{ f.title }}</h3>
      <p>{{ f.text }}</p>
    </article>
  </section>
</template>

<style scoped>
.features { display: grid; grid-template-columns: repeat(auto-fit, minmax(230px, 1fr)); gap: 1rem; padding: 3rem 0; scroll-margin-top: 4rem; }
.card { background: var(--surface); border: 1px solid var(--line); border-radius: 14px; padding: 1.4rem; }
h3 { margin: 0 0 0.5rem; font-size: 1.1rem; }
p { margin: 0; color: var(--muted); font-size: 0.95rem; }
</style>
