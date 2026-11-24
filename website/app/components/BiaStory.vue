<template>
  <main>
    <section class="page-hero">
      <div class="narrow">
        <nav class="crumbs" :aria-label="$t(`${ns}:breadcrumb`)">
          <NuxtLink :to="$lp('/')">{{ $t(`${ns}:home`) }}</NuxtLink><span aria-hidden="true">/</span>
          <NuxtLink :to="$lp('/cases')">{{ $t(`${ns}:real_cases`) }}</NuxtLink><span aria-hidden="true">/</span>
          <span>{{ $t(`${ns}:crumb`) }}</span>
        </nav>
        <span class="eyebrow">{{ $t(`${ns}:eyebrow`) }}</span>
        <h1>{{ $t(`${ns}:title`) }}</h1>
        <p class="section-lead">{{ $t(`${ns}:lead`) }}</p>
        <nav class="jump" :aria-label="$t(`${ns}:jump_to`)">
          <a class="chip" href="#story">{{ $t(`${ns}:jump_story`) }}</a>
          <a class="chip" href="#graph">{{ $t(`${ns}:jump_graph`) }}</a>
        </nav>
      </div>
    </section>

    <section id="story" class="section anchor">
      <div class="narrow story">
        <p class="story__byline">{{ $t(`${ns}:byline`, { date: writtenDate }) }}</p>
        <!-- Static, trusted text from ~/data/<bia>.ts. -->
        <p v-if="!noIntro" class="story__intro" v-html="$t(`${ns}:intro`)" />
        <article>
          <section v-for="c in story" :id="c.slug" :key="c.slug" class="story__chapter">
            <h2>{{ c.title }}</h2>
            <div v-for="(p, j) in c.body" :key="j" class="story__p" v-html="p.startsWith('<ul') ? p : `<p>${p}</p>`" />
          </section>
        </article>
        <p class="story__end">{{ $t(`${ns}:end`) }}</p>
      </div>
    </section>

    <section id="graph" class="section section--tint anchor">
      <div class="container">
        <span class="eyebrow">{{ $t(`${ns}:jump_graph`) }}</span>
        <h2 class="section-title">{{ $t(`${ns}:graph_title`) }}</h2>
        <p class="section-lead">{{ $t(`${ns}:graph_lead`) }}</p>
        <SocialGraph class="story-graph" :graph="graph" />
      </div>
    </section>

    <CtaBand :title="$t(`${ns}:cta_title`)" :text="$t(`${ns}:cta_text`)" />
  </main>
</template>

<script setup lang="ts">
// One real bia's page: its story in the first person, then its own social graph. The texts live in
// app/locales/<language>/<ns>.json; the chapters and the graph come from app/data/<bia>.ts.
import type { Chapter } from '~/data/fred'
import type { GraphData } from '~/data/graph'

// noIntro: the bia wrote no line before its first chapter
const props = defineProps<{ ns: string; story: Chapter[]; graph: GraphData; written: string; noIntro?: boolean }>()
const ns = props.ns
const locale = useLocale().value
const writtenDate = longDate(props.written, locale)
</script>

<style scoped>
.anchor { scroll-margin-top: calc(4.5rem + var(--strip-h, 0px)); }
.jump { display: flex; flex-wrap: wrap; gap: 0.45rem; margin-top: 1.5rem; }
.story-graph { margin-top: 2rem; }
.story__byline { font-family: var(--font-display); font-style: italic; font-size: 1.15rem; color: var(--ink-soft); }
.story__intro { color: var(--mute); font-size: 1rem; margin-bottom: 2.5rem; }
.story__chapter { scroll-margin-top: calc(5rem + var(--strip-h, 0px)); padding-top: 2.5rem; margin-top: 2.5rem; border-top: 1px solid var(--line); }
.story__chapter:first-child { border-top: 0; margin-top: 0; padding-top: 0; }
.story__chapter h2 { font-size: clamp(1.5rem, 3vw, 2rem); margin-bottom: 1rem; }
.story__p :deep(p), .story__p :deep(li) { font-size: 1.1rem; line-height: 1.75; }
.story__p :deep(ul) { margin: 0 0 1em; padding-left: 1.5rem; display: grid; gap: 0.6rem; }
.story__p :deep(em) { font-style: italic; color: var(--accent-deep); }
.story__p :deep(code), .story__intro :deep(code) { font-size: 0.9em; }
.story__chapter:first-child .story__p:first-of-type :deep(p)::first-letter {
  float: left; font-family: var(--font-display); font-weight: 700; font-size: 3.6rem; line-height: 0.9;
  padding: 0.35rem 0.5rem 0 0; color: var(--accent);
}
.story__end { margin-top: 2.5rem; text-align: right; font-family: var(--font-display); font-style: italic; font-size: 1.4rem; color: var(--ink-soft); }
</style>
