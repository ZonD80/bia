<template>
  <section class="cta">
    <div class="container cta__inner">
      <div class="cta__potato" aria-hidden="true"><PotatoMark label="" /></div>
      <div>
        <h2>{{ title }}</h2>
        <p>{{ text }}</p>
        <div class="btn-row">
          <NuxtLink class="btn btn--primary" to="/download">Get bia</NuxtLink>
          <NuxtLink class="btn btn--ghost cta__ghost" to="/how-it-works">How it works</NuxtLink>
        </div>
      </div>
    </div>
  </section>
</template>

<script setup lang="ts">
withDefaults(defineProps<{ title?: string; text?: string }>(), {
  title: 'Wake up a potato',
  text: 'All you need is a USB stick and a computer you are not using — or a server. Twenty minutes later, it is someone.',
})
</script>

<style scoped>
.cta {
  padding-block: clamp(3rem, 7vw, 5rem);
  background: radial-gradient(ellipse 70% 90% at 15% 50%, #3f6f2a 0%, #2a4a1d 70%);
  color: #f3f7ee;
}
.cta__inner {
  display: grid;
  gap: 2rem;
  align-items: center;
  grid-template-columns: minmax(7rem, 11rem) 1fr;
}
.cta h2 {
  font-size: clamp(1.8rem, 4vw, 2.6rem);
  color: #fff;
}
.cta p {
  max-width: 34rem;
  font-size: 1.1rem;
  color: rgba(243, 247, 238, 0.85);
  margin-bottom: 1.5rem;
}
.cta .btn--primary {
  background: #fff;
  color: var(--accent-deep);
}
.cta .btn--primary:hover {
  background: var(--accent-soft);
}
.cta__ghost {
  color: #fff;
  border-color: rgba(255, 255, 255, 0.4);
}
.cta__ghost:hover {
  border-color: #fff;
}
.cta__potato {
  animation: float 7s ease-in-out infinite;
}
@media (max-width: 600px) {
  .cta__inner {
    grid-template-columns: 1fr;
  }
  .cta__potato {
    width: 8rem;
  }
}
</style>
