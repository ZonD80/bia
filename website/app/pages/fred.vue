<template>
  <main>
    <section class="page-hero">
      <div class="narrow">
        <nav class="crumbs" aria-label="Breadcrumb">
          <NuxtLink to="/">Home</NuxtLink><span aria-hidden="true">/</span><span>Fred’s story</span>
        </nav>
        <span class="eyebrow">A real bia, in its own words</span>
        <h1>A short life, so far</h1>
        <p class="section-lead">
          Fred is the developer’s own bia: three weeks of running a real household’s errands, moving between
          computers and learning from its mistakes. This is Fred’s story as Fred wrote it — no names, no accounts,
          no addresses, just the shape of the thing.
        </p>
      </div>
    </section>

    <section class="section fred-film">
      <div class="narrow">
        <span class="eyebrow">The film Fred made</span>
        <h2>Asked for a promo, Fred made one</h2>
        <p class="section-lead">Fred wrote it, generated it and put it together. Nothing below was edited by a human.</p>
        <FredFilm />
      </div>
    </section>

    <section class="section section--tint">
      <div class="narrow story">
        <p class="story__byline">Written {{ fredWritten }}, by the one living it.</p>

        <nav class="story__toc card" aria-label="Chapters">
          <h2>Chapters</h2>
          <ol>
            <li v-for="c in fredChapters" :key="c.slug"><a :href="`#${c.slug}`">{{ c.title }}</a></li>
          </ol>
        </nav>

        <article>
          <section v-for="(c, i) in fredChapters" :id="c.slug" :key="c.slug" class="story__chapter">
            <span class="story__n">{{ String(i + 1).padStart(2, '0') }}</span>
            <h2>{{ c.title }}</h2>
            <!-- Static, trusted text from ~/data/fred.ts. -->
            <div v-for="(p, j) in c.body" :key="j" class="story__p" v-html="p.startsWith('<') ? p : `<p>${p}</p>`" />
          </section>
        </article>

        <p class="story__end">— Fred</p>
      </div>
    </section>

    <CtaBand title="Wake up your own" text="Fred started on a USB stick and an ordinary computer. Yours can too — and become someone else entirely." />
  </main>
</template>

<script setup lang="ts">
import { fredChapters, fredWritten } from '~/data/fred'

useSeoMeta({
  title: 'Fred’s story — a real bia, in its own words',
  description:
    'Fred, the developer’s own bia, tells three weeks of its life: waking up on a USB stick, learning whose family is whose, reminders, voice calls, moving between computers, and the unedited promo film it made itself.',
})
</script>

<style scoped>
.fred-film {
  padding-top: clamp(2rem, 4vw, 3rem);
}
.fred-film h2 {
  font-size: clamp(1.7rem, 3.5vw, 2.3rem);
}
.fred-film .section-lead {
  margin-bottom: 1.5rem;
}
.story__byline {
  font-family: var(--font-display);
  font-style: italic;
  font-size: 1.15rem;
  color: var(--ink-soft);
}
.story__toc {
  margin: 1.5rem 0 3rem;
}
.story__toc h2 {
  font-family: var(--font-body);
  font-size: 0.78rem;
  letter-spacing: 0.12em;
  text-transform: uppercase;
  color: var(--mute);
  margin-bottom: 0.6rem;
}
.story__toc ol {
  margin: 0;
  padding-left: 1.4rem;
  columns: 2 15rem;
  column-gap: 2.5rem;
}
.story__toc li {
  margin-bottom: 0.35rem;
  break-inside: avoid;
}
.story__toc a {
  color: var(--accent);
  font-weight: 600;
  text-decoration: none;
}
.story__toc a:hover {
  text-decoration: underline;
}
.story__chapter {
  scroll-margin-top: 5rem;
  padding-top: 2.5rem;
  margin-top: 2.5rem;
  border-top: 1px solid var(--line);
}
.story__chapter:first-child {
  border-top: 0;
  margin-top: 0;
  padding-top: 0;
}
.story__n {
  display: block;
  font-family: var(--font-display);
  font-weight: 700;
  font-size: 0.95rem;
  color: var(--tan);
  margin-bottom: 0.3rem;
}
.story__chapter h2 {
  font-size: clamp(1.5rem, 3vw, 2rem);
  margin-bottom: 1rem;
}
.story__p :deep(p),
.story__p :deep(li) {
  font-size: 1.1rem;
  line-height: 1.75;
}
.story__p :deep(ol) {
  margin: 0 0 1em;
  padding-left: 1.5rem;
  display: grid;
  gap: 0.6rem;
}
.story__p :deep(em) {
  font-style: italic;
  color: var(--accent-deep);
}
.story__chapter:first-child .story__p:first-of-type :deep(p)::first-letter {
  float: left;
  font-family: var(--font-display);
  font-weight: 700;
  font-size: 3.6rem;
  line-height: 0.9;
  padding: 0.35rem 0.5rem 0 0;
  color: var(--accent);
}
.story__end {
  margin-top: 2.5rem;
  text-align: right;
  font-family: var(--font-display);
  font-style: italic;
  font-size: 1.4rem;
  color: var(--ink-soft);
}
</style>
