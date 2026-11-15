<template>
  <main>
    <section class="page-hero">
      <div class="container">
        <nav class="crumbs" :aria-label="$t('usecases:breadcrumb')">
          <NuxtLink :to="$lp('/')">{{ $t('usecases:home') }}</NuxtLink><span aria-hidden="true">/</span><span>{{ $t('usecases:use_cases') }}</span>
        </nav>
        <span class="eyebrow">{{ $t('usecases:who_it_is_for') }}</span>
        <h1>{{ $t('usecases:one_bia_for_a_circle_of') }}</h1>
        <p class="section-lead">
          {{ $t('usecases:bia_is_not_a_personal_assistant') }}
        </p>
        <nav class="jump" :aria-label="$t('usecases:jump_to')">
          <a v-for="g in useCaseGroups" :key="g.slug" class="chip" :href="`#${g.slug}`">{{ g.title }}</a>
          <a class="chip" href="#your-own">{{ $t('usecases:your_own_use') }}</a>
        </nav>
      </div>
    </section>

    <section v-for="(g, i) in useCaseGroups" :id="g.slug" :key="g.slug" class="section ucg" :class="{ 'section--tint': i % 2 === 1 }">
      <div class="container">
        <div class="ucg__head">
          <span class="ucg__ico"><AppIcon :name="g.icon" size="2rem" /></span>
          <div>
            <h2>{{ g.title }}</h2>
            <p class="section-lead">{{ g.lead }}</p>
          </div>
        </div>

        <div class="grid grid--3 ucg__benefits">
          <div v-for="b in g.benefits" :key="b.title" class="card benefit">
            <h3>{{ b.title }}</h3>
            <p>{{ b.text }}</p>
          </div>
        </div>

        <div class="ucg__day" :style="{ '--dl-bg': i % 2 === 1 ? 'var(--paper-2)' : 'var(--paper)' }">
          <div class="ucg__daytext">
            <span class="eyebrow">{{ $t('usecases:one_day') }}</span>
            <h3>{{ g.day.title }}</h3>
            <p>{{ g.day.setting }}</p>
            <p class="ucg__key"><Tr k="usecases:bia_started_it_itself_someone_wrote"><i class="ucg__dot ucg__dot--own" /><i class="ucg__dot" /></Tr></p>
            <div class="chips">
              <NuxtLink v-for="u in g.uses" :key="u" class="chip" :to="$lp(`/features/${u}`)">{{ name(u) }}</NuxtLink>
            </div>
          </div>
          <DayLine :steps="g.day.steps" />
        </div>
      </div>
    </section>

    <section id="your-own" class="section section--sky own">
      <div class="container">
        <span class="eyebrow">{{ $t('usecases:your_own_use') }}</span>
        <h2 class="section-title">{{ $t('usecases:make_your_own_use_of_bia') }}</h2>
        <p class="section-lead">
          {{ $t('usecases:bia_is_social_and_has_a') }}
        </p>
        <ul class="list-check own__list">
          <li v-for="u in ownUses" :key="u">{{ u }}</li>
        </ul>
      </div>
    </section>

    <section class="section">
      <div class="narrow">
        <h2>{{ $t('usecases:a_word_about_using_it_well') }}</h2>
        <p>
          {{ $t('usecases:bia_talks_to_people_as_a') }}
        </p>
      </div>
    </section>

    <CtaBand :title="$t('usecases:give_your_circle_a_bia')" :text="$t('usecases:on_a_server_we_run_for')" />
  </main>
</template>

<script setup lang="ts">
const t = useT()
const { features } = useFeatures()
const { ownUses, useCaseGroups } = useUseCases()


useSeoMeta({
  title: t('usecases:use_cases_one_bia_for_a'),
  description:
    t('usecases:what_a_bia_changes_for_a'),
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
.ucg { padding-block: clamp(2.5rem, 6vw, 4rem); scroll-margin-top: calc(4.5rem + var(--strip-h, 0px)); }
.ucg__head {
  display: flex;
  gap: 1.1rem;
  align-items: flex-start;
  margin-bottom: 1.5rem;
}
.ucg__head h2 { font-size: clamp(1.6rem, 3.5vw, 2.2rem); margin-bottom: 0.3rem; }
.ucg__head .section-lead { max-width: 52rem; }
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
.benefit h3 { font-size: 1.1rem; margin-bottom: 0.35rem; }
.benefit p { margin: 0; color: var(--ink-soft); font-size: 0.96rem; }
.ucg__day {
  margin-top: 2.5rem;
  display: grid;
  gap: 2.5rem;
  grid-template-columns: 1fr 1.5fr;
  align-items: start;
}
.ucg__daytext { position: sticky; top: 6rem; }
.ucg__daytext h3 { font-size: clamp(1.4rem, 3vw, 1.8rem); margin: 0.2rem 0 0.5rem; }
.ucg__daytext p { color: var(--ink-soft); }
.ucg__key { font-size: 0.88rem; color: var(--mute) !important; display: flex; flex-wrap: wrap; align-items: center; gap: 0.35rem 0.5rem; }
.ucg__dot { width: 0.75rem; height: 0.75rem; border-radius: 50%; border: 2px solid var(--accent); background: #fff; display: inline-block; margin-left: 0.6rem; }
.ucg__dot:first-child { margin-left: 0; }
.ucg__dot--own { background: var(--accent); }
.own { scroll-margin-top: calc(4.5rem + var(--strip-h, 0px)); }
.own__list { margin-top: 1.5rem; display: grid; gap: 0.6rem 2rem; grid-template-columns: repeat(auto-fit, minmax(min(100%, 24rem), 1fr)); }
@media (max-width: 900px) {
  .ucg__day { grid-template-columns: 1fr; }
  .ucg__daytext { position: static; }
  .ucg__head { flex-direction: column; }
}
</style>
