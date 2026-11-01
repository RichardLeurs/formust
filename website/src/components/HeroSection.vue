<template>
  <section class="hero">
    <p class="eyebrow">Pascal reimagined with AI</p>
    <h1>A modern Pascal IDE<br />with coding agents for macOS</h1>
    <p class="lead">
      Pascoda wraps the Free Pascal compiler in a native, macOS-style IDE written in Rust.
      It builds console, GUI and Windows programs, designs forms with real controls, and
      has coding agents built in.
    </p>
    <div class="actions">
      <a class="btn primary" href="#start">Get started</a>
      <a class="btn" href="#features">See what it does</a>
    </div>
    <img class="shot" src="/pascoda.png" alt="Pascoda: the code editor, the AI assistant pane with a local Ollama model, and the terminal" width="1460" height="1034" />
  </section>
</template>

<style scoped>
.hero { text-align: center; padding: 4rem 0 2.5rem; }
.eyebrow { color: var(--accent); font-weight: 600; letter-spacing: 0.04em; text-transform: uppercase; font-size: 0.85rem; margin: 0; }
h1 { font-size: clamp(2.2rem, 5vw, 3.6rem); line-height: 1.1; letter-spacing: -0.02em; margin: 0.6rem 0 1rem; }
.lead { color: var(--muted); font-size: 1.15rem; max-width: 46rem; margin: 0 auto; }
.actions { display: flex; gap: 0.8rem; justify-content: center; margin: 2rem 0 3rem; }
.btn { padding: 0.65rem 1.25rem; border-radius: 999px; border: 1px solid var(--line); color: var(--text); text-decoration: none; font-weight: 500; }
.btn.primary { background: var(--accent); border-color: var(--accent); color: #fff; }
.shot {
  width: 100%;
  height: auto;
  border-radius: 12px;
  border: 1px solid var(--line);
  box-shadow: 0 30px 60px -20px rgba(0, 0, 0, 0.35);
}
</style>
