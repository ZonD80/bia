<template>
  <main>
    <section class="page-hero">
      <div class="container">
        <nav class="crumbs" :aria-label="$t('cases:breadcrumb')">
          <NuxtLink :to="$lp('/')">{{ $t('cases:home') }}</NuxtLink><span aria-hidden="true">/</span><span>{{ $t('cases:real_cases') }}</span>
        </nav>
        <span class="eyebrow">{{ $t('cases:eyebrow') }}</span>
        <h1>{{ $t('cases:title') }}</h1>
        <p class="section-lead">{{ $t('cases:lead') }}</p>
      </div>
    </section>

    <section class="section">
      <div class="container">
        <div class="grid grid--2 cases">
          <article v-for="c in cases" :key="c.to" class="card case">
            <span class="eyebrow">{{ c.who }}</span>
            <h2>{{ c.name }}</h2>
            <p class="case__text">{{ c.text }}</p>
            <ul class="case__facts">
              <li v-for="f in c.facts" :key="f">{{ f }}</li>
            </ul>
            <p class="case__date">{{ c.date }}</p>
            <NuxtLink class="btn btn--primary" :to="$lp(c.to)">{{ c.link }}</NuxtLink>
          </article>
        </div>
      </div>
    </section>

    <CtaBand :title="$t('cases:cta_title')" :text="$t('cases:cta_text')" />
  </main>
</template>

<script setup lang="ts">
import { fredNowFirst, fredNowWritten } from '~/data/fred'
import { jessicaWritten } from '~/data/jessica'
import { bobbyWritten } from '~/data/bobby'
import { provisionerWritten } from '~/data/provisioner'
import { suanWritten } from '~/data/suan'

const t = useT()
const locale = useLocale().value
const cases = [
  {
    to: '/fred',
    name: 'Fred',
    who: t('cases:fred_who'),
    text: t('cases:fred_text'),
    facts: [t('cases:fred_fact_1'), t('cases:fred_fact_2'), t('cases:fred_fact_3')],
    date: t('cases:updated', { date: longDate(fredNowWritten, locale), first: longDate(fredNowFirst, locale) }),
    link: t('cases:read_fred'),
  },
  {
    to: '/jessica',
    name: 'Jessica',
    who: t('cases:jessica_who'),
    text: t('cases:jessica_text'),
    facts: [t('cases:jessica_fact_1'), t('cases:jessica_fact_2'), t('cases:jessica_fact_3')],
    date: t('cases:written', { date: longDate(jessicaWritten, locale) }),
    link: t('cases:read_jessica'),
  },
  {
    to: '/bobby',
    name: 'Bobby',
    who: t('cases:bobby_who'),
    text: t('cases:bobby_text'),
    facts: [t('cases:bobby_fact_1'), t('cases:bobby_fact_2'), t('cases:bobby_fact_3')],
    date: t('cases:written', { date: longDate(bobbyWritten, locale) }),
    link: t('cases:read_bobby'),
  },
  {
    to: '/provisioner',
    name: 'Marketplace Provisioner',
    who: t('cases:provisioner_who'),
    text: t('cases:provisioner_text'),
    facts: [t('cases:provisioner_fact_1'), t('cases:provisioner_fact_2'), t('cases:provisioner_fact_3')],
    date: t('cases:written', { date: longDate(provisionerWritten, locale) }),
    link: t('cases:read_provisioner'),
  },
  {
    to: '/suan',
    name: '酸菜鱼',
    who: t('cases:suan_who'),
    text: t('cases:suan_text'),
    facts: [t('cases:suan_fact_1'), t('cases:suan_fact_2'), t('cases:suan_fact_3')],
    date: t('cases:written', { date: longDate(suanWritten, locale) }),
    link: t('cases:read_suan'),
  },
]

useSeoMeta({ title: t('cases:seo_title'), description: t('cases:seo_description') })
</script>

<style scoped>
.cases { align-items: stretch; }
.case { display: flex; flex-direction: column; gap: 0.6rem; }
.case h2 { font-size: clamp(1.8rem, 3.5vw, 2.4rem); margin: 0; }
.case__text { color: var(--ink-soft); font-size: 1.05rem; }
.case__facts { margin: 0; padding-left: 1.2rem; color: var(--ink-soft); display: grid; gap: 0.3rem; }
.case__date { color: var(--mute); font-size: 0.9rem; font-style: italic; margin-top: auto; }
.case .btn { align-self: flex-start; }
</style>
