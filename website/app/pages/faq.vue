<template>
  <main>
    <section class="page-hero">
      <div class="container">
        <nav class="crumbs" :aria-label="$t('faq:breadcrumb')">
          <NuxtLink :to="$lp('/')">{{ $t('faq:home') }}</NuxtLink><span aria-hidden="true">/</span><span>{{ $t('faq:faq') }}</span>
        </nav>
        <span class="eyebrow">{{ $t('faq:questions_people_ask') }}</span>
        <h1>{{ $t('faq:frequently_asked_questions') }}</h1>
      </div>
    </section>

    <section class="section">
      <div class="narrow">
        <div v-for="s in sections" :key="s.title" class="faqs">
          <h2>{{ s.title }}</h2>
          <details v-for="q in s.items" :key="q.q" class="faq">
            <summary>{{ q.q }}</summary>
            <p>{{ q.a }}</p>
          </details>
        </div>
      </div>
    </section>

    <CtaBand :title="$t('faq:still_curious')" :text="$t('faq:the_best_way_to_understand_bia')" />
  </main>
</template>

<script setup lang="ts">
const t = useT()
useSeoMeta({
  title: t('faq:faq_questions_about_bia'),
  description: t('faq:answers_about_bia_what_it_is'),
})

const sections = [
  {
    title: t('faq:the_basics'),
    items: [
      { q: t('faq:what_is_bia_in_one_sentence'), a: t('faq:a_free_operating_system_that_boots') },
      { q: t('faq:is_it_a_robot'), a: t('faq:it_has_no_body_but_it') },
      { q: t('faq:do_i_need_to_know_how'), a: t('faq:no_you_speak_to_bia_in') },
      { q: t('faq:what_does_it_cost'), a: t('faq:bia_itself_is_free_software_for') },
    ],
  },
  {
    title: t('faq:computers_and_internet'),
    items: [
      { q: t('faq:will_it_work_on_my_old'), a: t('faq:very_likely_bia_runs_on_most') },
      { q: t('faq:does_it_delete_windows'), a: t('faq:no_starting_from_the_stick_changes') },
      { q: t('faq:does_it_need_the_internet'), a: t('faq:normally_yes_messengers_and_the_cloud') },
      { q: t('faq:do_i_need_a_screen'), a: t('faq:only_for_the_first_minutes_after') },
      { q: t('faq:can_bia_live_on_a_rented'), a: t('faq:yes_log_in_to_a_linux') },
      { q: t('faq:can_my_bia_move_to_a'), a: t('faq:yes_give_it_ssh_access_to') },
    ],
  },
  {
    title: t('faq:talking_and_languages'),
    items: [
      { q: t('faq:which_languages_does_it_speak'), a: t('faq:bia_understands_speech_in_about_a') },
      { q: t('faq:can_it_answer_phone_calls'), a: t('faq:yes_telegram_and_whatsapp_voice_calls') },
      { q: t('faq:does_it_use_my_telegram_whatsapp'), a: t('faq:it_can_use_an_account_you') },
    ],
  },
  {
    title: t('faq:privacy_and_safety'),
    items: [
      { q: t('faq:where_are_my_conversations_stored'), a: t('faq:on_bias_own_disk_on_the') },
      { q: t('faq:who_can_talk_to_bia'), a: t('faq:anyone_you_let_reach_it_on') },
      { q: t('faq:is_the_cloud_brain_private'), a: t('faq:when_you_use_a_cloud_brain') },
      { q: t('faq:what_if_the_computer_is_stolen'), a: t('faq:bias_disk_holds_its_memories_and') },
    ],
  },
  {
    title: t('faq:using_it_responsibly'),
    items: [
      { q: t('faq:can_bia_pretend_to_be_me'), a: t('faq:it_can_write_from_an_account') },
      { q: t('faq:does_it_follow_the_messengers_rules'), a: t('faq:you_are_responsible_for_using_bia') },
      { q: t('faq:what_if_someone_tries_to_trick'), a: t('faq:bia_has_trust_levels_and_good') },
    ],
  },
]
</script>

<style scoped>
.faqs { margin-bottom: 2.5rem; }
.faqs h2 { font-size: 1.5rem; margin-bottom: 0.8rem; }
.faq {
  border: 1px solid var(--line);
  border-radius: 0.8rem;
  background: var(--card);
  margin-bottom: 0.6rem;
  padding: 0 1.1rem;
}
.faq summary {
  cursor: pointer;
  padding: 1rem 0;
  font-weight: 700;
  list-style: none;
  display: flex;
  justify-content: space-between;
  gap: 1rem;
}
.faq summary::-webkit-details-marker { display: none; }
.faq summary::after {
  content: '+';
  font-size: 1.3rem;
  line-height: 1;
  color: var(--accent);
}
.faq[open] summary::after { content: '–'; }
.faq p { color: var(--ink-soft); margin: 0 0 1rem; }
</style>
