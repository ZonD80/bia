<template>
  <main>
    <!-- hero -->
    <section class="hero">
      <div class="container hero__grid">
        <div class="hero__text rise">
          <span class="eyebrow">{{ $t('home:a_social_harness_agent') }}</span>
          <h1><Tr k="home:turn_anything_into_someone"><em /></Tr></h1>
          <p class="hero__lead">
            {{ $t('home:bia_is_an_ai_agent_with') }}
          </p>
          <div class="btn-row">
            <NuxtLink class="btn btn--primary" :to="$lp('/download')">{{ $t('home:get_bia') }}</NuxtLink>
            <a class="btn btn--ghost" href="#graph">{{ $t('home:see_a_real_ones_social_graph') }}</a>
          </div>
          <p class="hero__small">{{ $t('home:free_and_open_source_on_a') }}</p>
        </div>
        <div class="hero__art" aria-hidden="true">
          <div class="hero__glow" />
          <div class="hero__stage"><div class="hero__potato"><PotatoMark label="" /></div></div>
          <div class="hero__bubble hero__bubble--a">
            <Tr k="home:to_marta_whatsapp_mum_took_her"><span class="hero__to" /></Tr>
          </div>
          <div class="hero__bubble hero__bubble--b">
            <Tr k="home:a_customer_calls_in_german_ja"><span class="hero__to" /></Tr>
          </div>
          <div class="hero__bubble hero__bubble--c">
            <Tr k="home:to_the_choir_telegram_thursday_in"><span class="hero__to" /></Tr>
          </div>
        </div>
      </div>
    </section>

    <!-- social harness -->
    <section class="section">
      <div class="container">
        <span class="eyebrow">{{ $t('home:what_is_a_social_harness') }}</span>
        <h2 class="section-title">{{ $t('home:the_part_that_makes_an_ai') }}</h2>
        <p class="section-lead">
          <Tr k="home:a_harness_is_everything_around_an"><strong /></Tr>
        </p>
        <div class="grid grid--3 what">
          <div v-for="h in harness" :key="h.title" class="card">
            <AppIcon :name="h.icon" size="2rem" class="what__ico" />
            <h3>{{ h.title }}</h3>
            <p>{{ h.text }}</p>
          </div>
        </div>
        <div class="btn-row center">
          <NuxtLink class="btn btn--ghost" :to="$lp('/features')">{{ $t('home:everything_bia_can_do') }}</NuxtLink>
        </div>
      </div>
    </section>

    <!-- a real graph -->
    <section id="graph" class="section section--tint graph">
      <div class="container">
        <span class="eyebrow">{{ $t('home:a_real_one') }}</span>
        <h2 class="section-title">{{ $t('home:fred_and_the_thirty_people_around') }}</h2>
        <p class="section-lead">
          {{ $t('home:fred_is_the_developers_own_bia') }}
        </p>
        <SocialGraph class="graph__fig" />
        <div class="grid grid--3 graph__real">
          <div v-for="r in fredReal" :key="r.title" class="card">
            <span class="graph__with">{{ r.withWhom }}</span>
            <h3>{{ r.title }}</h3>
            <p>{{ r.text }}</p>
          </div>
        </div>
        <div class="btn-row center">
          <NuxtLink class="btn btn--primary" :to="$lp('/fred')">{{ $t('home:read_freds_story_in_his_words') }}</NuxtLink>
        </div>
      </div>
    </section>

    <!-- who it is for -->
    <section class="section">
      <div class="container">
        <span class="eyebrow">{{ $t('home:who_it_is_for') }}</span>
        <h2 class="section-title">{{ $t('home:one_bia_for_a_circle_of') }}</h2>
        <p class="section-lead">
          {{ $t('home:not_a_subscription_and_a_private') }}
        </p>
        <div class="tabs" role="tablist" :aria-label="$t('home:who_it_is_for')">
          <button
            v-for="g in useCaseGroups"
            :id="`tab-${g.slug}`"
            :key="g.slug"
            class="tab"
            :class="{ 'is-on': g.slug === tab }"
            role="tab"
            type="button"
            :aria-selected="g.slug === tab"
            :aria-controls="`panel-${g.slug}`"
            @click="tab = g.slug"
          >
            <AppIcon :name="g.icon" size="1.3rem" />{{ g.title }}
          </button>
        </div>
        <div
          v-for="g in useCaseGroups"
          v-show="g.slug === tab"
          :id="`panel-${g.slug}`"
          :key="g.slug"
          class="who"
          role="tabpanel"
          :aria-labelledby="`tab-${g.slug}`"
        >
          <div class="who__text">
            <h3>{{ g.intro }}</h3>
            <ul class="list-check">
              <li v-for="b in g.benefits.slice(0, 4)" :key="b.title"><strong>{{ b.title }}.</strong> {{ b.text }}</li>
            </ul>
            <NuxtLink class="btn btn--ghost" :to="$lp(`/use-cases#${g.slug}`)">{{ $t('home:more_for', { title: g.title.replace('For ', '') }) }}</NuxtLink>
          </div>
          <div>
            <p class="who__day"><strong>{{ g.day.title }}.</strong> {{ g.day.setting }}</p>
            <DayLine :steps="g.day.steps" />
          </div>
        </div>
      </div>
    </section>

    <!-- your own use -->
    <section class="section section--sky">
      <div class="container own">
        <div>
          <span class="eyebrow">{{ $t('home:your_own_use') }}</span>
          <h2 class="section-title">{{ $t('home:make_your_own_use_of_bia') }}</h2>
          <p class="section-lead">
            {{ $t('home:bia_is_social_and_can_do') }}
          </p>
          <div class="btn-row">
            <NuxtLink class="btn btn--ghost" :to="$lp('/use-cases#your-own')">{{ $t('home:more_ideas') }}</NuxtLink>
          </div>
        </div>
        <ul class="list-check">
          <li v-for="u in ownUses.slice(0, 5)" :key="u">{{ u }}</li>
        </ul>
      </div>
    </section>

    <!-- where it lives -->
    <section class="section">
      <div class="container">
        <span class="eyebrow">{{ $t('home:a_life_of_its_own') }}</span>
        <h2 class="section-title">{{ $t('home:it_lives_on_its_own_computer') }}</h2>
        <p class="section-lead">
          {{ $t('home:bia_is_not_an_app_on') }}
        </p>
        <div class="grid grid--3 what">
          <div v-for="l in life" :key="l.title" class="card">
            <AppIcon :name="l.icon" size="2rem" class="what__ico" />
            <h3>{{ l.title }}</h3>
            <p>{{ l.text }}</p>
          </div>
        </div>
        <div class="btn-row center">
          <NuxtLink class="btn btn--primary" :to="$lp('/download')">{{ $t('home:where_bia_can_live_and_how') }}</NuxtLink>
        </div>
      </div>
    </section>

    <!-- the film -->
    <section class="section section--tint">
      <div class="container fred">
        <div>
          <span class="eyebrow">{{ $t('home:made_by_fred') }}</span>
          <h2 class="section-title">{{ $t('home:asked_for_a_promo_he_made') }}</h2>
          <p class="section-lead">
            {{ $t('home:fred_woke_up_on_a_laptop') }}
          </p>
          <div class="btn-row">
            <NuxtLink class="btn btn--ghost" :to="$lp('/fred')">{{ $t('home:freds_story') }}</NuxtLink>
          </div>
        </div>
        <FredFilm />
      </div>
    </section>

    <CtaBand />
  </main>
</template>

<script setup lang="ts">
const t = useT()
const { ownUses, useCaseGroups } = useUseCases()


useSeoMeta({
  title: t('home:bia_a_social_harness_agent_turn'),
  description:
    t('home:bia_is_an_ai_agent_with_2'),
})

const tab = ref(useCaseGroups[0]!.slug)

const harness = [
  { icon: 'sparkle', title: t('home:an_identity_of_its_own'), text: t('home:a_name_a_character_its_own') },
  { icon: 'people', title: t('home:a_social_graph'), text: t('home:bia_keeps_its_own_map_of') },
  { icon: 'chat', title: t('home:every_channel_people_use'), text: t('home:whatsapp_telegram_messenger_instagram') },
  { icon: 'book', title: t('home:memory_of_each_relationship'), text: t('home:what_was_said_what_was_promised') },
  { icon: 'clock', title: t('home:initiative_and_manners'), text: t('home:it_writes_first_when_something_is') },
  { icon: 'terminal', title: t('home:a_computer_to_act_with'), text: t('home:talking_is_half_of_it_bia') },
]

// from Fred’s own account of his days (12 November 2026)
const fredReal = [
  { withWhom: t('home:anna_and_vasily_telegram'), title: t('home:part_of_a_familys_week'), text: t('home:fred_sends_his_owners_mother_a') },
  { withWhom: t('home:mikhail_telegram_email'), title: t('home:a_meeting_he_arranged_himself'), text: t('home:asked_to_set_up_a_conversation') },
  { withWhom: t('home:jessica_another_bia_their_own_link'), title: t('home:a_colleague_of_his_own_kind'), text: t('home:jessica_is_a_friends_bia_she') },
]

const life = [
  { icon: 'globe', title: t('home:anywhere'), text: t('home:bia_cloud_runs_one_for_you') },
  { icon: 'move', title: t('home:it_moves_as_itself'), text: t('home:bia_packs_its_memories_chats_accounts') },
  { icon: 'refresh', title: t('home:it_updates_and_repairs_itself'), text: t('home:it_installs_its_own_updates_checks') },
  { icon: 'puzzle', title: t('home:it_develops_itself'), text: t('home:when_a_tool_is_missing_bia') },
  { icon: 'link', title: t('home:it_sets_up_other_bias'), text: t('home:a_bia_can_take_a_rented') },
  { icon: 'shield', title: t('home:separate_from_your_life'), text: t('home:it_never_needs_the_keys_to') },
]
</script>

<style scoped>
.hero {
  position: relative;
  overflow: hidden;
  background: linear-gradient(180deg, var(--sky) 0%, var(--paper) 100%);
  padding-block: clamp(3rem, 8vw, 6rem) clamp(3rem, 7vw, 5rem);
}
.hero__grid {
  display: grid;
  gap: 2.5rem;
  align-items: center;
  grid-template-columns: 1.15fr 1fr;
}
.hero h1 {
  font-size: clamp(2.6rem, 6.5vw, 4.6rem);
  line-height: 1.02;
  max-width: 13ch;
}
.hero h1 em {
  font-style: italic;
  color: var(--accent);
}
.hero__lead {
  font-size: clamp(1.08rem, 2vw, 1.25rem);
  color: var(--ink-soft);
  max-width: 36rem;
  margin-bottom: 1.75rem;
}
.hero__small {
  margin-top: 1.1rem;
  font-size: 0.9rem;
  color: var(--mute);
}
.hero__art {
  position: relative;
  min-height: 22rem;
}
.hero__glow {
  position: absolute;
  inset: 10% 5%;
  background: radial-gradient(circle, rgba(199, 154, 92, 0.35), transparent 65%);
  filter: blur(20px);
}
.hero__stage {
  position: absolute;
  inset: 0;
  display: flex;
  align-items: center;
  justify-content: center;
}
.hero__potato {
  width: min(24rem, 85%);
  animation: float 8s ease-in-out infinite;
}
.hero__bubble {
  position: absolute;
  max-width: 15rem;
  padding: 0.65rem 0.9rem;
  border-radius: 1rem;
  background: #fff;
  border: 1px solid var(--line);
  box-shadow: var(--shadow);
  font-size: 0.9rem;
  line-height: 1.4;
  animation: rise 0.9s ease-out both;
}
.hero__to { display: block; font-size: 0.7rem; font-weight: 700; letter-spacing: 0.03em; text-transform: uppercase; color: var(--mute); margin-bottom: 0.15rem; }
.hero__bubble--a { top: 2%; left: 0; animation-delay: 0.4s; border-bottom-left-radius: 0.3rem; }
.hero__bubble--b { top: 38%; right: -2%; animation-delay: 0.8s; background: #dfeaf6; border-bottom-right-radius: 0.3rem; }
.hero__bubble--c { bottom: 2%; left: 8%; animation-delay: 1.2s; border-bottom-left-radius: 0.3rem; }

.what { margin-top: 2rem; }
@media (min-width: 901px) { .what { grid-template-columns: repeat(3, minmax(0, 1fr)); } }
.what__ico { color: var(--accent); margin-bottom: 0.8rem; }

.center { justify-content: center; margin-top: 2rem; }

@media (max-width: 900px) {
  .hero__grid, .who, .own { grid-template-columns: 1fr; }
  .hero__art { min-height: 18rem; order: -1; }
  .hero__bubble { font-size: 0.82rem; max-width: 12rem; }
  /* keep the potato's eye visible on narrow screens */
  .hero__bubble--b { top: auto; bottom: 31%; right: 0; }
}
.fred {
  display: grid;
  gap: 2.5rem;
  grid-template-columns: minmax(0, 1fr) minmax(0, 1.25fr);
  align-items: center;
}
@media (max-width: 860px) {
  .fred { grid-template-columns: 1fr; }
}
.graph { scroll-margin-top: calc(4.5rem + var(--strip-h, 0px)); }
.graph__fig { margin-top: 2rem; }
.graph__real { margin-top: 2rem; }
.graph__real h3 { font-size: 1.15rem; margin: 0.3rem 0 0.35rem; }
.graph__real p { margin: 0; color: var(--ink-soft); font-size: 0.96rem; }
.graph__with { font-size: 0.75rem; font-weight: 700; letter-spacing: 0.04em; text-transform: uppercase; color: var(--mute); }
.what h3 { font-size: 1.15rem; }
.what p { margin: 0; color: var(--ink-soft); font-size: 0.97rem; }

.tabs { display: flex; flex-wrap: wrap; gap: 0.5rem; margin-top: 1.75rem; }
.tab {
  appearance: none;
  display: inline-flex;
  align-items: center;
  gap: 0.5rem;
  padding: 0.6rem 1.1rem;
  border: 1px solid var(--line);
  border-radius: 999px;
  background: var(--card);
  color: var(--ink-soft);
  font: inherit;
  font-weight: 700;
  cursor: pointer;
  transition: border-color 0.2s ease, background 0.2s ease, color 0.2s ease;
}
.tab:hover { border-color: rgba(63, 111, 42, 0.4); }
.tab.is-on { background: var(--accent); border-color: var(--accent); color: #fff; }
.who {
  margin-top: 2rem;
  display: grid;
  gap: 2.5rem;
  grid-template-columns: 1fr 1.4fr;
  align-items: start;
}
.who__text h3 { font-size: clamp(1.25rem, 2.6vw, 1.6rem); margin-bottom: 1rem; }
.who__text .list-check { margin-bottom: 1.5rem; }
.who__text .list-check li { color: var(--ink-soft); }
.who__day { color: var(--ink-soft); margin: 0 0 1.5rem; }

.own { display: grid; gap: 2.5rem; grid-template-columns: 1fr 1.1fr; align-items: center; }
@media (max-width: 900px) {
  .who, .own { grid-template-columns: 1fr; }
  .hero__bubble { max-width: 13.5rem; }
}
</style>
