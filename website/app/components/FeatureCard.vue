<template>
  <NuxtLink class="fcard" :to="`/features/${feature.slug}`">
    <span class="fcard__icon"><AppIcon :name="feature.icon" size="1.6rem" /></span>
    <h3>{{ feature.name }}</h3>
    <p>{{ feature.tagline }}</p>
    <span class="fcard__more">Learn more <AppIcon name="arrow" size="1rem" /></span>
  </NuxtLink>
</template>

<script setup lang="ts">
import type { Feature } from '~/data/features'

defineProps<{ feature: Feature }>()
</script>

<style scoped>
.fcard {
  display: flex;
  flex-direction: column;
  gap: 0.35rem;
  height: 100%;
  padding: 1.4rem;
  background: var(--card);
  border: 1px solid var(--line);
  border-radius: var(--radius);
  box-shadow: var(--shadow);
  color: inherit;
  text-decoration: none;
  transition: transform 0.2s ease, box-shadow 0.2s ease, border-color 0.2s ease;
}
.fcard:hover {
  transform: translateY(-3px);
  border-color: rgba(63, 111, 42, 0.35);
  box-shadow: 0 2px 4px rgba(29, 33, 38, 0.05), 0 14px 32px rgba(29, 33, 38, 0.09);
}
.fcard__icon {
  display: inline-flex;
  align-items: center;
  justify-content: center;
  width: 2.8rem;
  height: 2.8rem;
  border-radius: 0.8rem;
  background: var(--accent-soft);
  color: var(--accent-deep);
  margin-bottom: 0.4rem;
}
.fcard h3 {
  font-size: 1.15rem;
  margin: 0;
}
.fcard p {
  margin: 0;
  color: var(--ink-soft);
  font-size: 0.97rem;
  flex: 1;
}
.fcard__more {
  display: inline-flex;
  align-items: center;
  gap: 0.3rem;
  margin-top: 0.6rem;
  font-size: 0.9rem;
  font-weight: 700;
  color: var(--accent);
}
</style>
