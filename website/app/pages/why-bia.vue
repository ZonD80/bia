<template>
  <main>
    <section class="page-hero">
      <div class="container">
        <nav class="crumbs" :aria-label="$t('why:breadcrumb')">
          <NuxtLink :to="$lp('/')">{{ $t('why:home') }}</NuxtLink><span aria-hidden="true">/</span><span>{{ $t('why:why_bia') }}</span>
        </nav>
        <span class="eyebrow">{{ $t('why:not_an_app_on_your_pc') }}</span>
        <h1>{{ $t('why:why_bia_is_different_from_the') }}</h1>
        <p class="section-lead">
          {{ $t('why:todays_ai_assistants_move_into_your') }}
        </p>
      </div>
    </section>

    <section class="section">
      <div class="container">
        <div class="compare" role="table" :aria-label="$t('why:typical_ai_assistant_compared_with_bia')">
          <div class="compare__row compare__row--head" role="row">
            <span role="columnheader" />
            <span role="columnheader">{{ $t('why:typical_ai_assistant') }}</span>
            <span role="columnheader">bia</span>
          </div>
          <div v-for="r in compare" :key="r.topic" class="compare__row" role="row">
            <span class="compare__topic" role="rowheader">{{ r.topic }}</span>
            <span role="cell">{{ r.them }}</span>
            <span role="cell" class="compare__bia">{{ r.bia }}</span>
          </div>
        </div>
      </div>
    </section>

    <section class="section section--tint">
      <div class="container">
        <h2 class="section-title">{{ $t('why:a_week_in_bias_life') }}</h2>
        <p class="section-lead">{{ $t('why:from_bias_own_notes_during_a') }}</p>
        <ol class="week">
          <li v-for="(d, i) in week" :key="i" class="card">
            <span class="week__day">{{ d.day }}</span>
            <h3>{{ d.title }}</h3>
            <p>{{ d.text }}</p>
          </li>
        </ol>
      </div>
    </section>

    <section class="section">
      <div class="container">
        <h2 class="section-title">{{ $t('why:character_not_just_capability') }}</h2>
        <div class="grid grid--3">
          <div v-for="c in character" :key="c.title" class="card">
            <AppIcon :name="c.icon" size="2rem" class="ico" />
            <h3>{{ c.title }}</h3>
            <p>{{ c.text }}</p>
          </div>
        </div>
      </div>
    </section>

    <section class="section section--tint">
      <div class="narrow">
        <h2>{{ $t('why:where_bia_comes_from') }}</h2>
        <p>
          {{ $t('why:bia_grew_out_of_two_earlier') }}
        </p>
      </div>
    </section>

    <CtaBand />
  </main>
</template>

<script setup lang="ts">
const t = useT()
useSeoMeta({
  title: t('why:why_bia_is_different_from_typical'),
  description:
    t('why:how_bia_differs_from_ai_assistants'),
})

const compare = [
  { topic: t('why:where_it_lives'), them: t('why:inside_your_pc_or_in_a'), bia: t('why:on_its_own_computer_separate_from') },
  { topic: t('why:your_files'), them: t('why:can_read_almost_everything_on_your'), bia: t('why:never_touches_your_pc_it_has') },
  { topic: t('why:identity'), them: t('why:acts_as_you_with_your_logins'), bia: t('why:its_own_name_accounts_phone_numbers') },
  { topic: t('why:rules'), them: t('why:boxed_in_by_a_vendors_sandbox'), bia: t('why:owns_its_whole_machine_the_limits') },
  { topic: t('why:if_something_leaks'), them: t('why:your_files_and_accounts_go_with'), bia: t('why:only_what_bia_itself_holds_your') },
  { topic: t('why:how_people_reach_it'), them: t('why:you_go_to_its_app_or'), bia: t('why:it_comes_to_whatsapp_telegram') },
  { topic: t('why:memory'), them: t('why:unknown_or_complex_memory_engines_with'), bia: t('why:a_simple_diary_and_reflections_kept') },
  { topic: t('why:doing_things'), them: t('why:installs_software_that_pollutes_your'), bia: t('why:installs_what_it_needs_on_its') },
  { topic: t('why:long_tasks'), them: t('why:usually_locked_until_the_task_is'), bia: t('why:works_in_the_background_and_keeps') },
  { topic: t('why:being_away'), them: t('why:messages_are_simply_missed'), bia: t('why:catches_up_on_everything_that_arrived') },
  { topic: t('why:languages'), them: t('why:text_in_many_languages'), bia: t('why:hears_and_speaks_them_on_calls') },
  { topic: t('why:your_agents_data'), them: t('why:whatever_it_reads_can_end_up'), bia: t('why:on_bias_own_disk_only_the') },
  { topic: t('why:hardware_changes'), them: t('why:dies_with_your_computer'), bia: t('why:moves_to_new_hardware_and_repairs') },
]

const week = [
  { day: t('why:day_0'), title: t('why:moves_off_the_stick'), text: t('why:one_sentence_move_to_the_disk') },
  { day: t('why:day_1'), title: t('why:builds_its_senses'), text: t('why:installed_its_browser_connected_its') },
  { day: t('why:day_2'), title: t('why:watches_without_nagging'), text: t('why:wrote_a_watcher_for_mail_and') },
  { day: t('why:day_3_4'), title: t('why:gets_a_face'), text: t('why:set_its_profile_photos_turned_on') },
  { day: t('why:day_5'), title: t('why:makes_a_phone_call'), text: t('why:placed_and_answered_voice_calls_joined') },
  { day: t('why:day_6_7'), title: t('why:holds_its_ground'), text: t('why:politely_refused_a_stranger_who') },
]

const character = [
  { icon: 'hand', title: t('why:knows_its_limits'), text: t('why:captchas_and_decisions_that_are_yours') },
  { icon: 'shield', title: t('why:holds_boundaries'), text: t('why:strangers_cannot_talk_it_into_giving') },
  { icon: 'people', title: t('why:treats_people_as_people'), text: t('why:it_remembers_corrections_introductions') },
]
</script>

<style scoped>
.compare {
  display: grid;
  border: 1px solid var(--line);
  border-radius: var(--radius);
  overflow: hidden;
  background: var(--card);
}
.compare__row {
  display: grid;
  grid-template-columns: 1fr 1.4fr 1.6fr;
  gap: 1rem;
  padding: 1rem 1.25rem;
  border-top: 1px solid var(--line);
  align-items: start;
}
.compare__row--head {
  border-top: 0;
  background: var(--paper-2);
  font-weight: 700;
  font-size: 0.9rem;
  color: var(--mute);
}
.compare__topic {
  font-weight: 700;
}
.compare__bia {
  color: var(--accent-deep);
  font-weight: 600;
}
.week {
  list-style: none;
  margin: 2rem 0 0;
  padding: 0;
  display: grid;
  gap: 1.1rem;
  grid-template-columns: repeat(auto-fill, minmax(min(100%, 19rem), 1fr));
}
.week__day {
  display: inline-block;
  margin-bottom: 0.5rem;
  font-size: 0.8rem;
  font-weight: 700;
  letter-spacing: 0.08em;
  text-transform: uppercase;
  color: #8a5f2c;
  background: var(--tan-soft);
  padding: 0.2rem 0.6rem;
  border-radius: 999px;
}
.ico { color: var(--accent); margin-bottom: 0.7rem; }
@media (max-width: 720px) {
  .compare__row { grid-template-columns: 1fr; gap: 0.3rem; }
  .compare__row--head { display: none; }
  .compare__row span[role='cell']::before { font-size: 0.75rem; font-weight: 700; color: var(--mute); display: block; }
  .compare__row span[role='cell']:nth-child(2)::before { content: 'Typical AI'; }
  .compare__row span[role='cell']:nth-child(3)::before { content: 'bia'; }
}
</style>
