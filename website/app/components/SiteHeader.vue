<template>
  <header class="top" :class="{ 'top--open': open }">
    <div class="container top__inner">
      <NuxtLink class="top__brand" to="/" aria-label="bia home" @click="open = false">
        <span class="top__potato"><PotatoMark label="" /></span>
        <span>bia</span>
      </NuxtLink>
      <button class="top__menu" type="button" :aria-expanded="open" aria-controls="primary-nav" @click="open = !open">
        <span class="sr-only">Menu</span>
        <span class="top__bars" aria-hidden="true"><i /><i /><i /></span>
      </button>
      <nav id="primary-nav" class="top__nav" aria-label="Primary" @click="open = false">
        <NuxtLink to="/features">Features</NuxtLink>
        <NuxtLink to="/use-cases">Use cases</NuxtLink>
        <NuxtLink to="/how-it-works">How it works</NuxtLink>
        <NuxtLink to="/why-bia">Why bia</NuxtLink>
        <NuxtLink to="/fred">Fred’s story</NuxtLink>
        <NuxtLink to="/faq">FAQ</NuxtLink>
        <a :href="REPO_URL" target="_blank" rel="noopener">GitHub</a>
        <NuxtLink class="btn btn--primary top__cta" to="/download">Get bia</NuxtLink>
      </nav>
    </div>
  </header>
</template>

<script setup lang="ts">
const open = ref(false)
</script>

<style scoped>
.top {
  position: sticky;
  top: 0;
  z-index: 30;
  background: rgba(251, 248, 242, 0.88);
  backdrop-filter: saturate(1.4) blur(12px);
  border-bottom: 1px solid var(--line);
}
.top__inner {
  display: flex;
  align-items: center;
  justify-content: space-between;
  gap: 1rem;
  min-height: 4rem;
}
.top__brand {
  display: inline-flex;
  align-items: center;
  gap: 0.5rem;
  font-family: var(--font-display);
  font-weight: 700;
  font-size: 1.6rem;
  letter-spacing: -0.03em;
  color: var(--ink);
  text-decoration: none;
}
.top__potato {
  width: 2.3rem;
  margin-top: -0.4rem;
}
.top__nav {
  display: flex;
  align-items: center;
  gap: 0.3rem 1.4rem;
  font-weight: 600;
  font-size: 0.97rem;
}
.top__nav a:not(.btn) {
  color: var(--ink-soft);
  text-decoration: none;
}
.top__nav a:not(.btn):hover,
.top__nav a.router-link-active:not(.btn) {
  color: var(--accent);
}
.top__cta {
  min-height: 2.5rem;
  padding: 0.45rem 1.1rem;
  font-size: 0.92rem;
}
.top__menu {
  display: none;
  appearance: none;
  border: 0;
  background: transparent;
  padding: 0.5rem;
  cursor: pointer;
}
.top__bars {
  display: grid;
  gap: 5px;
}
.top__bars i {
  display: block;
  width: 22px;
  height: 2px;
  background: var(--ink);
  border-radius: 2px;
  transition: transform 0.2s ease, opacity 0.2s ease;
}
.top--open .top__bars i:nth-child(1) { transform: translateY(7px) rotate(45deg); }
.top--open .top__bars i:nth-child(2) { opacity: 0; }
.top--open .top__bars i:nth-child(3) { transform: translateY(-7px) rotate(-45deg); }

@media (max-width: 860px) {
  .top__menu {
    display: block;
  }
  .top__nav {
    position: absolute;
    top: 100%;
    left: 0;
    right: 0;
    flex-direction: column;
    align-items: stretch;
    gap: 0;
    padding: 0.5rem 1.25rem 1.25rem;
    background: var(--paper);
    border-bottom: 1px solid var(--line);
    box-shadow: 0 12px 24px rgba(29, 33, 38, 0.08);
    display: none;
  }
  .top--open .top__nav {
    display: flex;
  }
  .top__nav a:not(.btn) {
    padding: 0.85rem 0.25rem;
    border-bottom: 1px solid var(--line);
    font-size: 1.05rem;
  }
  .top__cta {
    margin-top: 1rem;
  }
}
</style>
