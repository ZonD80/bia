<template>
  <footer class="foot">
    <div class="container foot__grid">
      <div class="foot__brand">
        <div class="foot__row">
          <span class="foot__potato"><PotatoMark label="" /></span>
          <p class="foot__name">bia</p>
        </div>
        <p class="foot__motto">Boot into agent. Making every potato smart — on a global scale.</p>
      </div>
      <nav class="foot__col" aria-label="Talks with people">
        <h3>Talks</h3>
        <NuxtLink v-for="f in byGroup('talk')" :key="f.slug" :to="`/features/${f.slug}`">{{ f.name }}</NuxtLink>
      </nav>
      <nav class="foot__col" aria-label="Thinks and remembers">
        <h3>Thinks</h3>
        <NuxtLink v-for="f in byGroup('mind')" :key="f.slug" :to="`/features/${f.slug}`">{{ f.name }}</NuxtLink>
      </nav>
      <nav class="foot__col" aria-label="Gets things done">
        <h3>Does</h3>
        <NuxtLink v-for="f in byGroup('hands')" :key="f.slug" :to="`/features/${f.slug}`">{{ f.name }}</NuxtLink>
      </nav>
      <nav class="foot__col" aria-label="Lives and grows">
        <h3>Lives</h3>
        <NuxtLink v-for="f in byGroup('life')" :key="f.slug" :to="`/features/${f.slug}`">{{ f.name }}</NuxtLink>
      </nav>
      <nav class="foot__col" aria-label="More">
        <h3>More</h3>
        <NuxtLink to="/use-cases">Use cases</NuxtLink>
        <NuxtLink to="/how-it-works">How it works</NuxtLink>
        <NuxtLink to="/why-bia">Why bia</NuxtLink>
        <NuxtLink to="/faq">FAQ</NuxtLink>
        <NuxtLink to="/download">Get bia</NuxtLink>
      </nav>
    </div>
    <div class="container foot__base">
      <span>bia is free software (GPL-3.0). Use it kindly: tell people when they are talking to an assistant.</span>
    </div>
  </footer>
</template>

<script setup lang="ts">
import { features, type Feature } from '~/data/features'

const byGroup = (g: Feature['group']) => features.filter((f) => f.group === g)
</script>

<style scoped>
.foot {
  background: #1d2126;
  color: rgba(251, 248, 242, 0.72);
  padding-block: 3.5rem 2rem;
}
.foot__grid {
  display: grid;
  gap: 2rem;
  grid-template-columns: repeat(auto-fit, minmax(min(100%, 11rem), 1fr));
}
.foot__brand {
  grid-column: span 2;
  min-width: min(100%, 16rem);
}
.foot__row {
  display: flex;
  align-items: center;
  gap: 0.6rem;
}
.foot__potato {
  width: 3rem;
}
.foot__name {
  margin: 0;
  font-family: var(--font-display);
  font-weight: 700;
  font-size: 2rem;
  color: #fff;
}
.foot__motto {
  margin-top: 0.75rem;
  max-width: 22rem;
}
.foot__col {
  display: flex;
  flex-direction: column;
  gap: 0.45rem;
  font-size: 0.92rem;
}
.foot__col h3 {
  font-family: var(--font-body);
  font-size: 0.78rem;
  letter-spacing: 0.12em;
  text-transform: uppercase;
  color: #fff;
  margin-bottom: 0.4rem;
}
.foot__col a {
  color: rgba(251, 248, 242, 0.72);
  text-decoration: none;
}
.foot__col a:hover {
  color: #fff;
}
.foot__base {
  margin-top: 2.5rem;
  padding-top: 1.5rem;
  border-top: 1px solid rgba(255, 255, 255, 0.1);
  font-size: 0.86rem;
}
@media (max-width: 640px) {
  .foot__brand {
    grid-column: 1 / -1;
  }
}
</style>
