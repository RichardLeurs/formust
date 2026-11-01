<script setup>
import { editor } from '../data/content.js'
</script>

<template>
  <section class="editor">
    <h2>The editor and the tools around it</h2>
    <ul>
      <li v-for="item in editor" :key="item">{{ item }}</li>
    </ul>
  </section>
</template>

<style scoped>
.editor { padding: 2rem 0 3rem; }
h2 { font-size: 1.8rem; letter-spacing: -0.01em; margin: 0 0 1rem; }
ul { margin: 0; padding-left: 1.2rem; color: var(--muted); display: grid; gap: 0.5rem; }
li::marker { color: var(--accent); }
</style>
