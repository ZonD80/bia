<template>
  <main>
    <section class="page-hero">
      <div class="container">
        <nav class="crumbs" :aria-label="$t('privacy:breadcrumb')">
          <NuxtLink :to="$lp('/')">{{ $t('privacy:home') }}</NuxtLink><span aria-hidden="true">/</span><span>{{ $t('privacy:privacy') }}</span>
        </nav>
        <span class="eyebrow">{{ $t('privacy:privacy_policy') }}</span>
        <h1>{{ $t('privacy:we_collect_no_data') }}</h1>
        <p class="section-lead">
          {{ $t('privacy:not_on_this_website_not_in', { UPDATED }) }}
        </p>
      </div>
    </section>

    <section class="section">
      <div class="narrow policy">
        <div v-for="s in sections" :key="s.title">
          <h2>{{ s.title }}</h2>
          <p v-for="(p, i) in s.text" :key="i">{{ p }}</p>
        </div>
        <h2>{{ $t('privacy:questions') }}</h2>
        <p>
          <Tr k="privacy:ask_in_the_projects_issues_on" :v="{ REPO_URL: REPO_URL.replace('https://', '') }"><a :href="`${REPO_URL}/issues`" target="_blank" rel="noopener" /></Tr>
        </p>
      </div>
    </section>
  </main>
</template>

<script setup lang="ts">
const t = useT()
import { REPO_URL } from '~/composables/useLatestRelease'


const UPDATED = longDate('2026-10-28', useLocale().value)

useSeoMeta({
  title: t('privacy:privacy_policy_bia'),
  description: t('privacy:bia_the_bia_deploy_apps_mac'),
})

const sections = [
  {
    title: t('privacy:this_website'),
    text: [
      t('privacy:ownbia_com_sets_no_cookies_runs'),
      t('privacy:like_any_website_it_is_delivered'),
    ],
  },
  {
    title: t('privacy:bia_deploy_mac_windows_linux_android'),
    text: [
      t('privacy:the_app_needs_no_account_and'),
      t('privacy:your_ai_key_goes_to_the'),
      t('privacy:an_ssh_key_the_app_makes'),
      t('privacy:the_app_does_not_save_your'),
    ],
  },
  {
    title: t('privacy:bia_itself'),
    text: [
      t('privacy:bia_runs_on_a_computer_or'),
      t('privacy:to_work_bia_talks_to_the'),
    ],
  },
  {
    title: t('privacy:changes'),
    text: [t('privacy:if_this_ever_changes_this_page')],
  },
]
</script>

<style scoped>
.policy h2 {
  margin-top: 2rem;
}
.policy p {
  margin-top: 0.75rem;
}
</style>
