<script setup>
import HeroSection from './components/HeroSection.vue'
import FeatureGrid from './components/FeatureGrid.vue'
import EditorList from './components/EditorList.vue'
import StatusTable from './components/StatusTable.vue'
import GetStarted from './components/GetStarted.vue'
</script>

<template>
  <header class="top">
    <strong>Pascoda</strong>
    <nav>
      <a href="#features">Features</a>
      <a href="#status">Status</a>
      <a href="#start">Get started</a>
      <a href="/documentation/" target="_blank" rel="noopener">Documentation</a>
    </nav>
  </header>

  <main>
    <HeroSection />
    <FeatureGrid />
    <EditorList />
    <StatusTable />
    <GetStarted />
  </main>

  <footer>
    <p>Pascoda is a modern Pascal IDE for macOS, written in Rust. Free Pascal is used under the GPL and LGPL.</p>
  </footer>
</template>

<style scoped>
.top {
  position: sticky;
  top: 0;
  z-index: 10;
  display: flex;
  justify-content: space-between;
  align-items: center;
  padding: 0.8rem 2rem;
  background: color-mix(in srgb, var(--bg) 85%, transparent);
  backdrop-filter: blur(12px);
  border-bottom: 1px solid var(--line);
}
nav { display: flex; gap: 1.5rem; }
nav a { color: var(--muted); text-decoration: none; font-size: 0.95rem; }
nav a:hover { color: var(--text); }
main { max-width: 1080px; margin: 0 auto; padding: 0 1.5rem; }
footer { text-align: center; color: var(--muted); font-size: 0.85rem; padding: 3rem 1rem; }
</style>
