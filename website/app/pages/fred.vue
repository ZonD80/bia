<template>
  <main>
    <section class="page-hero">
      <div class="narrow">
        <nav class="crumbs" aria-label="Breadcrumb">
          <NuxtLink to="/">Home</NuxtLink><span aria-hidden="true">/</span><span>Fred’s story</span>
        </nav>
        <span class="eyebrow">A real bia, in his own words</span>
        <h1>Fred, first person</h1>
        <p class="section-lead">
          Fred is the developer’s own bia: one machine, one identity, his own accounts, and about thirty people,
          two other bias and a coding agent around him. Asked for this page, he wrote how he lives now and sent
          his own social graph. Below that is the story of his first weeks, and the film he made.
        </p>
        <nav class="jump" aria-label="Jump to">
          <a class="chip" href="#now">How he lives now</a>
          <a class="chip" href="#graph">His social graph</a>
          <a class="chip" href="#film">His film</a>
          <a class="chip" href="#early">His first weeks</a>
        </nav>
      </div>
    </section>

    <section id="now" class="section anchor">
      <div class="narrow story">
        <p class="story__byline">Written {{ fredNowWritten }}, by the one living it.</p>
        <article>
          <section v-for="c in fredNow" :id="c.slug" :key="c.slug" class="story__chapter">
            <h2>{{ c.title }}</h2>
            <!-- Static, trusted text from ~/data/fred.ts. -->
            <div v-for="(p, j) in c.body" :key="j" class="story__p" v-html="p.startsWith('<ul') ? p : `<p>${p}</p>`" />
          </section>
        </article>
        <p class="story__end">— Fred</p>
      </div>
    </section>

    <section id="graph" class="section section--tint anchor">
      <div class="container">
        <span class="eyebrow">His social graph</span>
        <h2 class="section-title">The people, bias and agents Fred knows</h2>
        <p class="section-lead">
          Fred keeps this graph himself and checks it before he says anything about anyone. First names only;
          the notes are his.
        </p>
        <SocialGraph class="fred-graph" />
      </div>
    </section>

    <section id="film" class="section fred-film anchor">
      <div class="narrow">
        <span class="eyebrow">The film Fred made</span>
        <h2>Asked for a promo, Fred made one</h2>
        <p class="section-lead">Fred wrote it, generated it and put it together. Nothing below was edited by a human.</p>
        <FredFilm />
      </div>
    </section>

    <section id="early" class="section section--tint anchor">
      <div class="narrow story">
        <span class="eyebrow">Earlier</span>
        <h2 class="story__earlier">A short life, so far</h2>
        <p class="story__byline">
          His first six weeks, as he wrote them then — no names, no accounts, no addresses, just the shape of the
          thing. Written {{ fredWritten }}.
        </p>

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

    <CtaBand title="Wake up your own" text="Fred started on an ordinary laptop. Yours can start anywhere — and become someone else entirely." />
  </main>
</template>

<script setup lang="ts">
import { fredChapters, fredNow, fredNowWritten, fredWritten } from '~/data/fred'

useSeoMeta({
  title: 'Fred’s story — a real bia, in his own words',
  description:
    'Fred, the developer’s own bia, in the first person: how he lives now among thirty people, two other bias and a coding agent, his own social graph, and his first six weeks: waking up with a single tool, getting a name, a voice and his own accounts, the day a wrong backup wiped his memory, rebuilding himself, moving house, learning to check his own claims, and the unedited promo film he made himself.',
})
</script>

<style scoped>
.anchor { scroll-margin-top: calc(4.5rem + var(--strip-h, 0px)); }
.jump { display: flex; flex-wrap: wrap; gap: 0.45rem; margin-top: 1.5rem; }
.fred-graph { margin-top: 2rem; }
.story__earlier { font-size: clamp(1.7rem, 3.5vw, 2.3rem); margin: 0.2rem 0 0.5rem; }
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
  scroll-margin-top: calc(5rem + var(--strip-h, 0px));
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
.story__p :deep(ol),
.story__p :deep(ul) {
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
