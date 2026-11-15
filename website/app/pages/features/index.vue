<template>
  <main>
    <section class="page-hero">
      <div class="container">
        <nav class="crumbs" :aria-label="$t('features:breadcrumb')">
          <NuxtLink :to="$lp('/')">{{ $t('features:home') }}</NuxtLink><span aria-hidden="true">/</span><span>{{ $t('features:features') }}</span>
        </nav>
        <span class="eyebrow">{{ $t('features:everything_bia_does') }}</span>
        <h1>{{ $t('features:one_personality_20_abilities_and') }}</h1>
        <p class="section-lead">
          {{ $t('features:bia_is_easier_to_understand_by') }}
        </p>
      </div>
    </section>

    <section v-for="(g, key) in groups" :key="key" class="section group" :class="{ 'section--tint': key === 'mind' || key === 'life' }">
      <div class="container">
        <h2 class="group__title">{{ g.title }}</h2>
        <p class="section-lead">{{ g.blurb }}</p>
        <div class="grid grid--feat">
          <FeatureCard v-for="f in byGroup(key)" :key="f.slug" :feature="f" />
        </div>
      </div>
    </section>

    <CtaBand :title="$t('features:ready_to_meet_it')" :text="$t('features:write_bia_onto_a_usb_stick')" />
  </main>
</template>

<script setup lang="ts">
const t = useT()
import type { Feature } from '~/data/features'

const { features, groups } = useFeatures()


useSeoMeta({
  title: t('features:features_everything_bia_can_do'),
  description:
    t('features:all_of_bias_abilities_messengers_voice'),
})

const byGroup = (g: string) => features.filter((f) => f.group === (g as Feature['group']))
</script>

<style scoped>
.group { padding-block: clamp(2.5rem, 6vw, 4rem); }
.group__title { font-size: clamp(1.6rem, 3.5vw, 2.2rem); }
.group .section-lead { margin-bottom: 1.5rem; }
</style>
