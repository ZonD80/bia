<template>
  <main v-if="feature">
    <section class="page-hero">
      <div class="container fhero">
        <div>
          <nav class="crumbs" aria-label="Breadcrumb">
            <NuxtLink to="/">Home</NuxtLink><span aria-hidden="true">/</span>
            <NuxtLink to="/features">Features</NuxtLink><span aria-hidden="true">/</span>
            <span>{{ feature.name }}</span>
          </nav>
          <span class="eyebrow">{{ groups[feature.group].title }}</span>
          <h1>{{ feature.name }}</h1>
          <p class="fhero__tag">{{ feature.tagline }}</p>
        </div>
        <span class="fhero__icon" aria-hidden="true"><AppIcon :name="feature.icon" size="3.5rem" /></span>
      </div>
    </section>

    <section class="section intro">
      <div class="container intro__grid">
        <div>
          <p class="intro__lead">{{ feature.lead }}</p>
          <h2 class="h-small">What it means for you</h2>
          <div class="points">
            <div v-for="p in feature.points" :key="p.title" class="point">
              <h3>{{ p.title }}</h3>
              <p>{{ p.text }}</p>
            </div>
          </div>
        </div>
        <aside class="intro__aside">
          <ChatDemo :lines="feature.chat" title="For example" />
        </aside>
      </div>
    </section>

    <section class="section section--tint">
      <div class="container ideas">
        <div>
          <h2 class="h-small">Ideas for using it</h2>
          <ul class="list-check">
            <li v-for="e in feature.examples" :key="e">{{ e }}</li>
          </ul>
        </div>
        <div>
          <h2 class="h-small">Good to know</h2>
          <p class="note-box">{{ feature.good_to_know }}</p>
        </div>
      </div>
    </section>

    <section class="section">
      <div class="container">
        <h2 class="h-small">Works hand in hand with</h2>
        <div class="grid grid--feat">
          <FeatureCard v-for="r in related" :key="r.slug" :feature="r" />
        </div>
        <nav class="pager" aria-label="More features">
          <NuxtLink v-if="prev" :to="`/features/${prev.slug}`" class="pager__link">← {{ prev.name }}</NuxtLink>
          <span v-else />
          <NuxtLink v-if="next" :to="`/features/${next.slug}`" class="pager__link pager__link--next">{{ next.name }} →</NuxtLink>
        </nav>
      </div>
    </section>

    <CtaBand />
  </main>
</template>

<script setup lang="ts">
import { features, groups, getFeature, type Feature } from '~/data/features'

const route = useRoute()
const feature = computed(() => getFeature(String(route.params.slug)))

if (!feature.value) {
  throw createError({ statusCode: 404, statusMessage: 'Feature not found', fatal: true })
}

const related = computed(() =>
  (feature.value?.related ?? []).map((s) => getFeature(s)).filter((f): f is Feature => !!f),
)
const index = computed(() => features.findIndex((f) => f.slug === feature.value?.slug))
const prev = computed(() => (index.value > 0 ? features[index.value - 1] : undefined))
const next = computed(() => (index.value < features.length - 1 ? features[index.value + 1] : undefined))

useSeoMeta({
  title: () => `${feature.value?.name} — bia`,
  description: () => `${feature.value?.tagline} ${feature.value?.lead}`.slice(0, 300),
})
</script>

<style scoped>
.fhero {
  display: flex;
  justify-content: space-between;
  align-items: flex-end;
  gap: 2rem;
}
.fhero__tag {
  font-size: clamp(1.1rem, 2.2vw, 1.35rem);
  color: var(--ink-soft);
  max-width: 40rem;
  margin: 0;
}
.fhero__icon {
  display: inline-flex;
  align-items: center;
  justify-content: center;
  width: 6.5rem;
  height: 6.5rem;
  border-radius: 1.5rem;
  background: var(--accent-soft);
  color: var(--accent-deep);
  flex-shrink: 0;
}
.intro__grid {
  display: grid;
  gap: 2.5rem;
  grid-template-columns: 1.3fr 1fr;
  align-items: start;
}
.intro__lead {
  font-size: clamp(1.1rem, 2vw, 1.25rem);
  line-height: 1.65;
  margin-bottom: 2rem;
}
.intro__aside {
  position: sticky;
  top: 5.5rem;
}
.h-small {
  font-size: clamp(1.35rem, 2.6vw, 1.7rem);
  margin-bottom: 1rem;
}
.points {
  display: grid;
  gap: 1.25rem;
}
.point {
  padding-left: 1rem;
  border-left: 3px solid var(--accent-soft);
}
.point h3 {
  font-size: 1.1rem;
  margin-bottom: 0.3rem;
}
.point p {
  margin: 0;
  color: var(--ink-soft);
}
.ideas {
  display: grid;
  gap: 2.5rem;
  grid-template-columns: 1.2fr 1fr;
}
.pager {
  display: flex;
  justify-content: space-between;
  gap: 1rem;
  margin-top: 2.5rem;
  padding-top: 1.5rem;
  border-top: 1px solid var(--line);
}
.pager__link {
  font-weight: 700;
  text-decoration: none;
}
@media (max-width: 860px) {
  .intro__grid,
  .ideas {
    grid-template-columns: 1fr;
  }
  /* one column: the aside would stick over the text while it scrolls */
  .intro__aside {
    position: static;
  }
  .intro__aside {
    position: static;
  }
  .fhero__icon {
    display: none;
  }
}
</style>
