<template>
  <main>
    <section class="page-hero">
      <div class="container">
        <nav class="crumbs" :aria-label="$t('how:breadcrumb')">
          <NuxtLink :to="$lp('/')">{{ $t('how:home') }}</NuxtLink><span aria-hidden="true">/</span><span>{{ $t('how:how_it_works') }}</span>
        </nav>
        <span class="eyebrow">{{ $t('how:no_technical_knowledge_needed') }}</span>
        <h1>{{ $t('how:how_bia_works_in_plain_words') }}</h1>
        <p class="section-lead">
          {{ $t('how:a_normal_computer_starts_up_into') }}
        </p>
      </div>
    </section>

    <section class="section">
      <div class="container">
        <h2 class="section-title">{{ $t('how:from_a_usb_stick_to_a') }}</h2>
        <ol class="flow">
          <li v-for="(s, i) in flow" :key="i" class="flow__item">
            <span class="flow__n">{{ i + 1 }}</span>
            <div>
              <h3>{{ s.title }}</h3>
              <p>{{ s.text }}</p>
            </div>
          </li>
        </ol>
      </div>
    </section>

    <section class="section section--tint">
      <div class="container two">
        <div>
          <h2 class="section-title">{{ $t('how:one_conversation_for_everything') }}</h2>
          <p class="section-lead">
            {{ $t('how:everything_reaches_bia_the_same_way') }}
          </p>
        </div>
        <ChatDemo :lines="inbox" :title="$t('how:what_bia_sees')" />
      </div>
    </section>

    <section class="section">
      <div class="container">
        <h2 class="section-title">{{ $t('how:whats_inside_bia') }}</h2>
        <div class="grid grid--3">
          <div v-for="b in inside" :key="b.title" class="card">
            <AppIcon :name="b.icon" size="2rem" class="ico" />
            <h3>{{ b.title }}</h3>
            <p>{{ b.text }}</p>
          </div>
        </div>
      </div>
    </section>

    <section class="section section--tint">
      <div class="container two">
        <div>
          <h2 class="section-title">{{ $t('how:what_you_need') }}</h2>
          <ul class="list-check">
            <li>{{ $t('how:a_usb_stick_of_8_gb') }}</li>
            <li>{{ $t('how:a_computer_almost_any_pc_or') }}</li>
            <li>{{ $t('how:for_messengers_any_of_a_telegram') }}</li>
            <li>{{ $t('how:for_the_cloud_brain_an_internet') }}</li>
            <li>{{ $t('how:optional_for_the_brain_on_the') }}</li>
          </ul>
        </div>
        <div>
          <h2 class="section-title">{{ $t('how:what_you_dont_need') }}</h2>
          <ul class="list-check">
            <li>{{ $t('how:no_programming_you_talk_to_bia') }}</li>
            <li>{{ $t('how:no_monitor_once_it_runs_use') }}</li>
            <li>{{ $t('how:no_subscription_to_bia_itself_it') }}</li>
            <li>{{ $t('how:no_cloud_account_keeps_your_agents') }}</li>
          </ul>
        </div>
      </div>
    </section>

    <CtaBand />
  </main>
</template>

<script setup lang="ts">
const t = useT()
useSeoMeta({
  title: t('how:how_bia_works_in_plain_words_2'),
  description:
    t('how:how_bia_turns_a_usb_stick'),
})

const flow = [
  { title: t('how:write_bia_onto_a_usb_stick'), text: t('how:download_the_small_bia_deploy_app') },
  { title: t('how:start_a_computer_from_the_stick'), text: t('how:plug_it_into_a_pc_or') },
  { title: t('how:say_hello'), text: t('how:type_on_the_screen_or_open') },
  { title: t('how:give_it_a_life'), text: t('how:ask_bia_to_join_whatsapp_telegram') },
  { title: t('how:let_it_settle_in'), text: t('how:move_bia_from_the_stick_onto') },
]

const inbox = [
  { from: 'you' as const, where: t('how:screen'), text: t('how:which_disks_are_in_this_computer') },
  { from: 'them' as const, where: t('how:whatsapp_maria'), text: t('how:is_there_parking_at_the_guesthouse') },
  { from: 'note' as const, text: t('how:a_reminder_is_due_call_the') },
  { from: 'note' as const, text: t('how:a_background_job_finished_photos') },
  { from: 'bia' as const, where: t('how:everywhere_at_once'), text: t('how:answers_maria_on_whatsapp_tells_you') },
]

const inside = [
  { icon: 'brain', title: t('how:a_brain'), text: t('how:a_connection_to_an_ai_language') },
  { icon: 'chat', title: t('how:its_senses_and_voice'), text: t('how:messengers_calls_speech_in_about_a') },
  { icon: 'terminal', title: t('how:its_hands'), text: t('how:bia_is_a_complete_modern_operating') },
  { icon: 'key', title: t('how:its_safe'), text: t('how:a_vault_for_passwords_two_factor') },
  { icon: 'book', title: t('how:its_memory'), text: t('how:a_diary_reflections_an_address_book') },
  { icon: 'refresh', title: t('how:its_survival_kit'), text: t('how:tools_to_back_itself_up_move') },
]
</script>

<style scoped>
.flow {
  list-style: none;
  margin: 2rem 0 0;
  padding: 0;
  display: grid;
  gap: 1.25rem;
  counter-reset: s;
}
.flow__item {
  display: grid;
  grid-template-columns: 3.2rem 1fr;
  gap: 1rem;
  align-items: start;
  padding: 1.25rem;
  background: var(--card);
  border: 1px solid var(--line);
  border-radius: var(--radius);
}
.flow__item h3 { font-size: 1.2rem; margin-bottom: 0.3rem; }
.flow__item p { margin: 0; color: var(--ink-soft); }
.flow__n {
  display: inline-flex;
  align-items: center;
  justify-content: center;
  width: 3rem;
  height: 3rem;
  border-radius: 50%;
  background: var(--accent);
  color: #fff;
  font-family: var(--font-display);
  font-weight: 700;
  font-size: 1.3rem;
}
.two {
  display: grid;
  gap: 2.5rem;
  grid-template-columns: 1fr 1fr;
  align-items: start;
}
.ico { color: var(--accent); margin-bottom: 0.7rem; }
@media (max-width: 860px) {
  .two { grid-template-columns: 1fr; }
}
</style>
