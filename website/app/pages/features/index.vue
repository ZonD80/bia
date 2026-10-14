<template>
  <main>
    <section class="page-hero">
      <div class="container">
        <nav class="crumbs" aria-label="Breadcrumb">
          <NuxtLink to="/">Home</NuxtLink><span aria-hidden="true">/</span><span>Features</span>
        </nav>
        <span class="eyebrow">Everything bia does</span>
        <h1>One personality, twenty abilities</h1>
        <p class="section-lead">
          bia is easier to understand by what it does for people than by how it works inside. Every ability below
          has its own page with a sample conversation and everyday ideas.
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

    <CtaBand title="Ready to meet it?" text="Write bia onto a USB stick and wake up a computer you are not using." />
  </main>
</template>

<script setup lang="ts">
import { features, groups, type Feature } from '~/data/features'

useSeoMeta({
  title: 'Features — everything bia can do',
  description:
    'All of bia’s abilities: messengers, voice calls, 100 languages, its own chat app, memory, calendar, a real computer and browser, a password vault, moving between computers, self-updates, talking to other agents.',
})

const byGroup = (g: string) => features.filter((f) => f.group === (g as Feature['group']))
</script>

<style scoped>
.group { padding-block: clamp(2.5rem, 6vw, 4rem); }
.group__title { font-size: clamp(1.6rem, 3.5vw, 2.2rem); }
.group .section-lead { margin-bottom: 1.5rem; }
</style>
