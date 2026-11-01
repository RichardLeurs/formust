<script setup>
import { requirements, buildSteps } from '../data/content.js'
</script>

<template>
  <section id="start" class="start">
    <h2>Get started</h2>
    <h3>Requirements</h3>
    <ul>
      <li v-for="r in requirements" :key="r">{{ r }}</li>
    </ul>
    <h3>Build from source</h3>
    <pre><code>{{ buildSteps.join('\n') }}</code></pre>
    <p class="muted">
      The app bundles Free Pascal, the Lazarus LCL and the language server, so a built
      <code>Pascoda.app</code> needs only the Command Line Tools. Setup notes for a checkout are in the
      repository's README.md and in <code>vitepress/</code>, the documentation site.
    </p>
  </section>
</template>

<style scoped>
.start { padding: 3rem 0 4rem; scroll-margin-top: 4rem; }
h2 { font-size: 1.8rem; letter-spacing: -0.01em; margin: 0 0 1rem; }
h3 { font-size: 1.05rem; margin: 1.5rem 0 0.5rem; }
ul { margin: 0; padding-left: 1.2rem; color: var(--muted); display: grid; gap: 0.4rem; }
pre { background: var(--surface); border: 1px solid var(--line); border-radius: 10px; padding: 1rem; overflow-x: auto; }
pre code { display: block; background: none; padding: 0; white-space: pre; }
.muted { color: var(--muted); font-size: 0.95rem; }
</style>
