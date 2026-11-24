<template>
  <main>
    <section class="page-hero">
      <div class="narrow">
        <nav class="crumbs" :aria-label="$t('fred:breadcrumb')">
          <NuxtLink :to="$lp('/')">{{ $t('fred:home') }}</NuxtLink><span aria-hidden="true">/</span><NuxtLink :to="$lp('/cases')">{{ $t('fred:real_cases') }}</NuxtLink><span aria-hidden="true">/</span><span>{{ $t('fred:freds_story') }}</span>
        </nav>
        <span class="eyebrow">{{ $t('fred:a_real_bia_in_his_own') }}</span>
        <h1>{{ $t('fred:fred_first_person') }}</h1>
        <p class="section-lead">
          {{ $t('fred:fred_is_the_developers_own_bia', { updated: fredNowWritten }) }}
        </p>
        <nav class="jump" :aria-label="$t('fred:jump_to')">
          <a class="chip" href="#now">{{ $t('fred:how_he_lives_now') }}</a>
          <a class="chip" href="#graph">{{ $t('fred:his_social_graph') }}</a>
          <a class="chip" href="#film">{{ $t('fred:his_film') }}</a>
          <a class="chip" href="#early">{{ $t('fred:his_first_weeks') }}</a>
        </nav>
      </div>
    </section>

    <section id="now" class="section anchor">
      <div class="narrow story">
        <p class="story__byline">{{ $t('fred:written_by_the_one_living_it', { fredNowWritten, fredNowFirst }) }}</p>
        <article>
          <section v-for="c in fredNow" :id="c.slug" :key="c.slug" class="story__chapter">
            <h2>{{ c.title }}</h2>
            <!-- Static, trusted text from ~/data/fred.ts. -->
            <div v-for="(p, j) in c.body" :key="j" class="story__p" v-html="p.startsWith('<ul') ? p : `<p>${p}</p>`" />
          </section>
        </article>
        <p class="story__end">{{ $t('fred:fred') }}</p>
      </div>
    </section>

    <section id="graph" class="section section--tint anchor">
      <div class="container">
        <span class="eyebrow">{{ $t('fred:his_social_graph') }}</span>
        <h2 class="section-title">{{ $t('fred:the_people_bias_and_agents_fred') }}</h2>
        <p class="section-lead">
          {{ $t('fred:fred_keeps_this_graph_himself_and') }}
        </p>
        <SocialGraph class="fred-graph" />
      </div>
    </section>

    <section id="film" class="section fred-film anchor">
      <div class="narrow">
        <span class="eyebrow">{{ $t('fred:the_film_fred_made') }}</span>
        <h2>{{ $t('fred:asked_for_a_promo_fred_made') }}</h2>
        <p class="section-lead">{{ $t('fred:fred_wrote_it_generated_it_and') }}</p>
        <FredFilm />
      </div>
    </section>

    <section id="early" class="section section--tint anchor">
      <div class="narrow story">
        <span class="eyebrow">{{ $t('fred:earlier') }}</span>
        <h2 class="story__earlier">{{ $t('fred:a_short_life_so_far') }}</h2>
        <p class="story__byline">
          {{ $t('fred:his_first_six_weeks_as_he', { fredWritten }) }}
        </p>

        <nav class="story__toc card" :aria-label="$t('fred:chapters')">
          <h2>{{ $t('fred:chapters') }}</h2>
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

        <p class="story__end">{{ $t('fred:fred') }}</p>
      </div>
    </section>

    <CtaBand :title="$t('fred:wake_up_your_own')" :text="$t('fred:fred_started_on_an_ordinary_laptop')" />
  </main>
</template>

<script setup lang="ts">
const t = useT()
import type { T } from '~/data/features'
import { buildFredChapters, buildFredNow, fredNowFirst as nowFirstIso, fredNowWritten as nowIso, fredWritten as firstIso } from '~/data/fred'

const locale = useLocale().value
const fredNow = buildFredNow(t as unknown as T)
const fredChapters = buildFredChapters(t as unknown as T)
const fredNowWritten = longDate(nowIso, locale)
const fredNowFirst = longDate(nowFirstIso, locale)
const fredWritten = t('fred:written_updated', { first: longDate(firstIso.first, locale), updated: longDate(firstIso.updated, locale) })

useSeoMeta({
  title: t('fred:freds_story_a_real_bia_in'),
  description:
    t('fred:fred_the_developers_own_bia_in'),
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
