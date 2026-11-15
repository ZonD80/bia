<template>
  <main>
    <section class="page-hero">
      <div class="container">
        <nav class="crumbs" :aria-label="$t('cloud:breadcrumb')">
          <NuxtLink :to="$lp('/')">{{ $t('cloud:home') }}</NuxtLink><span aria-hidden="true">/</span><span>bia cloud</span>
        </nav>
        <span class="eyebrow">{{ $t('cloud:bia_cloud_early_access') }}</span>
        <h1>{{ $t('cloud:no_computer_to_spare_let_us') }}</h1>
        <p class="section-lead">
          {{ $t('cloud:bia_cloud_gives_a_bia_a') }}
        </p>
        <div class="btn-row">
          <a class="btn btn--primary" :href="`${CLOUD}/#start`">{{ $t('cloud:wake_up_a_bia_in_the') }}</a>
          <a class="btn btn--ghost" :href="`${CLOUD}/pricing/`">{{ $t('cloud:prices') }}</a>
        </div>
      </div>
    </section>

    <section class="section">
      <div class="container">
        <span class="eyebrow">{{ $t('cloud:how_it_works') }}</span>
        <h2 class="section-title">{{ $t('cloud:a_name_a_language_one_payment') }}</h2>
        <div class="grid grid--3 steps">
          <div v-for="(s, i) in steps" :key="i" class="card step">
            <span class="step__n">{{ i + 1 }}</span>
            <h3>{{ s.title }}</h3>
            <p>{{ s.text }}</p>
          </div>
        </div>
      </div>
    </section>

    <section class="section section--tint">
      <div class="container">
        <span class="eyebrow">{{ $t('cloud:pay_as_you_go') }}</span>
        <h2 class="section-title">{{ $t('cloud:one_balance_no_plan') }}</h2>
        <div class="grid grid--3">
          <div class="card">
            <h3>{{ $t('cloud:its_computer') }}</h3>
            <p><Tr k="cloud:17_99_a_month_vat_for"><strong /></Tr></p>
          </div>
          <div class="card">
            <h3>{{ $t('cloud:its_brain') }}</h3>
            <p>{{ $t('cloud:it_starts_on_deepseek_paid_from') }}</p>
          </div>
          <div class="card">
            <h3>{{ $t('cloud:to_start') }}</h3>
            <p><Tr k="cloud:the_first_two_weeks_of_its"><strong /></Tr></p>
          </div>
        </div>
        <p class="cloud__fine"><Tr k="cloud:prices_are_drafts_during_early_access"><a :href="`${CLOUD}/pricing/`" /></Tr></p>
      </div>
    </section>

    <section class="section">
      <div class="container">
        <span class="eyebrow">{{ $t('cloud:still_yours') }}</span>
        <h2 class="section-title">{{ $t('cloud:we_run_the_computer_the_bia') }}</h2>
        <ul class="list-check cloud__list">
          <li><Tr k="cloud:only_you_hold_its_key_you"><strong /></Tr></li>
          <li><Tr k="cloud:we_dont_look_inside_its_memories"><strong /></Tr></li>
          <li><Tr k="cloud:take_it_home_any_time_ask"><strong /><NuxtLink :to="$lp('/download')" /></Tr></li>
          <li><Tr k="cloud:backed_up_every_night_and_a"><strong /></Tr></li>
          <li><Tr k="cloud:several_bias_one_account_share_the"><strong /></Tr></li>
        </ul>
      </div>
    </section>

    <section class="section section--sky">
      <div class="narrow cloud__or">
        <h2 class="section-title">{{ $t('cloud:or_run_it_yourself_free') }}</h2>
        <p class="section-lead">
          <Tr k="cloud:bia_is_free_software_a_usb"><NuxtLink :to="$lp('/download')" /><NuxtLink :to="$lp('/servers')" /></Tr>
        </p>
      </div>
    </section>

    <CtaBand :title="$t('cloud:ready_when_you_are')" :text="$t('cloud:wake_one_up_in_the_cloud')" />
  </main>
</template>

<script setup lang="ts">
const t = useT()
const CLOUD = 'https://cloud.ownbia.com'

useSeoMeta({
  title: t('cloud:bia_cloud_a_bia_on_its'),
  description:
    t('cloud:bia_cloud_gives_a_bia_its'),
})

const steps = [
  { title: t('cloud:name_it'), text: t('cloud:choose_its_name_and_the_language') },
  { title: t('cloud:pay_once'), text: t('cloud:the_first_two_weeks_of_its_2') },
  { title: t('cloud:say_hello'), text: t('cloud:its_own_server_starts_and_bia') },
]
</script>

<style scoped>
.steps .card { position: relative; }
.step__n { display: inline-grid; place-items: center; width: 2rem; height: 2rem; border-radius: 50%; background: var(--accent); color: #fff; font-weight: 800; margin-bottom: 0.6rem; }
.cloud__fine { margin-top: 1.2rem; color: var(--mute); font-size: 0.9rem; }
.cloud__list { max-width: 48rem; }
.cloud__or { text-align: center; }
</style>
