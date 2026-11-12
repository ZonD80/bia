<template>
  <ol class="dl">
    <li v-for="(s, i) in steps" :key="i" :class="{ 'dl__own': s.own }">
      <span class="dl__time">{{ s.time }}</span>
      <div>
        <p class="dl__head">
          <strong>{{ s.who }}</strong>
          <span class="dl__where">{{ s.where }}</span>
          <span v-if="s.own" class="dl__tag" title="bia started this itself">on its own</span>
        </p>
        <p class="dl__what">{{ s.what }}</p>
      </div>
    </li>
  </ol>
</template>

<script setup lang="ts">
import type { DayStep } from '~/data/usecases'

defineProps<{ steps: DayStep[] }>()
</script>

<style scoped>
.dl {
  list-style: none;
  margin: 0;
  padding: 0 0 0 1.4rem;
  display: grid;
  gap: 1.15rem;
  border-left: 2px solid rgba(63, 111, 42, 0.25);
}
.dl li {
  display: grid;
  grid-template-columns: 3.4rem 1fr;
  gap: 0.75rem;
  position: relative;
}
.dl li::before {
  content: '';
  position: absolute;
  left: -1.85rem;
  top: 0.4rem;
  width: 0.8rem;
  height: 0.8rem;
  border-radius: 50%;
  background: #fff;
  border: 2px solid var(--accent);
  box-shadow: 0 0 0 4px var(--dl-bg, var(--paper));
}
.dl li.dl__own::before { background: var(--accent); }
.dl__time {
  font-family: var(--font-display);
  font-weight: 600;
  color: var(--accent-deep);
}
.dl__head {
  margin: 0 0 0.2rem;
  display: flex;
  flex-wrap: wrap;
  align-items: baseline;
  gap: 0.2rem 0.6rem;
}
.dl__where { color: var(--mute); font-size: 0.9rem; }
.dl__tag {
  font-size: 0.72rem;
  font-weight: 700;
  letter-spacing: 0.04em;
  text-transform: uppercase;
  color: var(--accent-deep);
  background: var(--accent-soft);
  border-radius: 999px;
  padding: 0.1rem 0.55rem;
}
.dl__what { margin: 0; color: var(--ink-soft); }
@media (max-width: 560px) {
  .dl li { grid-template-columns: 1fr; gap: 0.1rem; }
}
</style>
