<template>
  <figure class="demo" :aria-label="title || 'Example conversation'">
    <figcaption v-if="title" class="demo__title">{{ title }}</figcaption>
    <div class="demo__body">
      <template v-for="(l, i) in lines" :key="i">
        <p v-if="l.from === 'note'" class="demo__note">{{ l.text }}</p>
        <div v-else class="demo__row" :class="`demo__row--${l.from}`">
          <span class="demo__who">
            {{ l.from === 'bia' ? 'bia' : l.from === 'you' ? 'You' : 'Someone' }}<template v-if="l.where"> · {{ l.where }}</template>
          </span>
          <p class="demo__bubble">{{ l.text }}</p>
        </div>
      </template>
    </div>
  </figure>
</template>

<script setup lang="ts">
import type { ChatLine } from '~/data/features'

defineProps<{ lines: ChatLine[]; title?: string }>()
</script>

<style scoped>
.demo {
  margin: 0;
  background: linear-gradient(180deg, #eef3ea, #f7f4ec);
  border: 1px solid var(--line);
  border-radius: 1.25rem;
  padding: 1.25rem;
  box-shadow: var(--shadow);
}
.demo__title {
  font-size: 0.8rem;
  font-weight: 700;
  letter-spacing: 0.1em;
  text-transform: uppercase;
  color: var(--mute);
  margin-bottom: 0.9rem;
}
.demo__body {
  display: flex;
  flex-direction: column;
  gap: 0.7rem;
}
.demo__row {
  display: flex;
  flex-direction: column;
  max-width: 85%;
}
.demo__row--you,
.demo__row--them {
  align-self: flex-end;
  align-items: flex-end;
}
.demo__row--bia {
  align-self: flex-start;
}
.demo__who {
  font-size: 0.74rem;
  font-weight: 700;
  color: var(--mute);
  margin: 0 0.6rem 0.2rem;
}
.demo__bubble {
  margin: 0;
  padding: 0.6rem 0.9rem;
  border-radius: 1rem;
  line-height: 1.45;
  font-size: 0.98rem;
}
.demo__row--bia .demo__bubble {
  background: #fff;
  border: 1px solid var(--line);
  border-top-left-radius: 0.3rem;
}
.demo__row--you .demo__bubble {
  background: var(--accent);
  color: #fff;
  border-top-right-radius: 0.3rem;
}
.demo__row--them .demo__bubble {
  background: #dfeaf6;
  border-top-right-radius: 0.3rem;
}
.demo__note {
  align-self: center;
  margin: 0.1rem 0;
  font-size: 0.82rem;
  font-style: italic;
  color: var(--mute);
  text-align: center;
  max-width: 90%;
}
</style>
