<template>
  <main>
    <section class="page-hero">
      <div class="container">
        <nav class="crumbs" aria-label="Breadcrumb">
          <NuxtLink to="/">Home</NuxtLink><span aria-hidden="true">/</span><span>Use cases</span>
        </nav>
        <span class="eyebrow">Ideas from real life</span>
        <h1>What people can do with bia</h1>
        <p class="section-lead">
          bia is a general helper, so the list is endless. These stories show how its abilities combine — pick
          the ones that sound like your life.
        </p>
        <nav class="jump" aria-label="Jump to">
          <a v-for="g in useCaseGroups" :key="g.slug" class="chip" :href="`#${g.slug}`">{{ g.title }}</a>
        </nav>
      </div>
    </section>

    <section v-for="(g, i) in useCaseGroups" :id="g.slug" :key="g.slug" class="section ucg" :class="{ 'section--tint': i % 2 === 1 }">
      <div class="container">
        <div class="ucg__head">
          <span class="ucg__ico"><AppIcon :name="g.icon" size="2rem" /></span>
          <div>
            <h2>{{ g.title }}</h2>
            <p class="section-lead">{{ g.intro }}</p>
          </div>
        </div>
        <div class="grid grid--2">
          <article v-for="c in g.cases" :key="c.title" class="card story">
            <h3>{{ c.title }}</h3>
            <p>{{ c.story }}</p>
            <div class="chips">
              <NuxtLink v-for="u in c.uses" :key="u" class="chip" :to="`/features/${u}`">{{ name(u) }}</NuxtLink>
            </div>
          </article>
        </div>
      </div>
    </section>

    <section class="section">
      <div class="narrow">
        <h2>A word about using it well</h2>
        <p>
          bia talks to people as a person would, which is exactly what makes it useful — and why it should be used
          honestly. Tell people when they are talking to an assistant where that matters, respect the rules of
          each messenger, ask before acting for someone else, and keep bia’s computer as safe as you keep your
          own phone.
        </p>
      </div>
    </section>

    <CtaBand title="Your story could be next" text="Write bia onto a stick, wake up an old computer, and give it a job." />
  </main>
</template>

<script setup lang="ts">
import { features } from '~/data/features'
import { useCaseGroups } from '~/data/usecases'

useSeoMeta({
  title: 'Use cases — what people do with bia',
  description:
    'Ways to use bia for families, small businesses, travellers, communities, freelancers, schools, places with little internet and old computers.',
})

const name = (slug: string) => features.find((f) => f.slug === slug)?.name ?? slug
</script>

<style scoped>
.jump {
  display: flex;
  flex-wrap: wrap;
  gap: 0.45rem;
  margin-top: 1.5rem;
}
.ucg { padding-block: clamp(2.5rem, 6vw, 4rem); scroll-margin-top: 4.5rem; }
.ucg__head {
  display: flex;
  gap: 1.1rem;
  align-items: flex-start;
  margin-bottom: 1.5rem;
}
.ucg__head h2 { font-size: clamp(1.6rem, 3.5vw, 2.2rem); margin-bottom: 0.3rem; }
.ucg__ico {
  display: inline-flex;
  align-items: center;
  justify-content: center;
  width: 3.5rem;
  height: 3.5rem;
  border-radius: 1rem;
  background: var(--tan-soft);
  color: #8a5f2c;
  flex-shrink: 0;
}
.story { display: flex; flex-direction: column; }
.story p { color: var(--ink-soft); flex: 1; }
</style>
