<script setup>
import { status } from '../data/content.js'
</script>

<template>
  <section id="status" class="status">
    <h2>Status</h2>
    <p class="note">Early work in progress. Most of it is tested; mouse and keyboard use in the real window has been checked less than the code.</p>
    <table>
      <thead>
        <tr><th>Area</th><th>State</th><th>Notes</th></tr>
      </thead>
      <tbody>
        <tr v-for="row in status" :key="row.area">
          <td>{{ row.area }}</td>
          <td><span class="state" :class="row.state">{{ row.state }}</span></td>
          <td>{{ row.notes }}</td>
        </tr>
      </tbody>
    </table>
  </section>
</template>

<style scoped>
.status { padding: 3rem 0; scroll-margin-top: 4rem; }
h2 { font-size: 1.8rem; letter-spacing: -0.01em; margin: 0 0 0.5rem; }
.note { color: var(--muted); margin: 0 0 1.2rem; }
table { width: 100%; border-collapse: collapse; background: var(--surface); border: 1px solid var(--line); border-radius: 12px; overflow: hidden; font-size: 0.95rem; }
th, td { text-align: left; padding: 0.75rem 1rem; border-bottom: 1px solid var(--line); vertical-align: top; }
th { color: var(--muted); font-weight: 500; font-size: 0.85rem; }
tr:last-child td { border-bottom: none; }
.state { font-size: 0.8rem; padding: 0.15rem 0.55rem; border-radius: 999px; background: var(--code-bg); }
.state.working { color: #15803d; }
.state.partly { color: #b45309; }
</style>
