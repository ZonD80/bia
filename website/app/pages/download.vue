<template>
  <main>
    <section class="page-hero">
      <div class="container">
        <nav class="crumbs" :aria-label="$t('download:breadcrumb')">
          <NuxtLink :to="$lp('/')">{{ $t('download:home') }}</NuxtLink><span aria-hidden="true">/</span><span>{{ $t('download:get_bia') }}</span>
        </nav>
        <span class="eyebrow">{{ $t('download:get_bia') }}</span>
        <h1>{{ $t('download:give_a_bia_a_place_to') }}</h1>
        <p class="section-lead">
          {{ $t('download:a_bia_is_not_an_app') }}
        </p>
        <nav class="jump" :aria-label="$t('download:jump_to')">
          <a class="chip" href="#cloud">bia cloud</a>
          <a class="chip" href="#installers">{{ $t('download:installers') }}</a>
          <a class="chip" href="#life">{{ $t('download:how_a_bia_lives') }}</a>
          <a class="chip" href="#stick">{{ $t('download:usb_stick_step_by_step') }}</a>
        </nav>
      </div>
    </section>

    <!-- 1. cloud -->
    <section id="cloud" class="section anchor">
      <div class="container">
        <div class="cloud card">
          <div>
            <span class="eyebrow">{{ $t('download:the_easy_way') }}</span>
            <h2>{{ $t('download:bia_cloud_we_run_its_computer') }}</h2>
            <p>
              {{ $t('download:give_it_a_name_and_a') }}
            </p>
            <ul class="list-check">
              <li>{{ $t('download:from_17_99_a_month_for') }}</li>
              <li>{{ $t('download:thinking_is_paid_as_it_is') }}</li>
              <li>{{ $t('download:nothing_to_install_you_talk_to') }}</li>
            </ul>
            <div class="btn-row">
              <a class="btn btn--primary" :href="'https://cloud.ownbia.com' + $lp('/')" target="_blank" rel="noopener">{{ $t('download:start_a_bia_in_the_cloud') }}</a>
              <NuxtLink class="btn btn--ghost" :to="$lp('/cloud')">{{ $t('download:about_bia_cloud') }}</NuxtLink>
            </div>
          </div>
          <div class="cloud__art" aria-hidden="true"><PotatoMark label="" /></div>
        </div>
      </div>
    </section>

    <!-- 2. installers -->
    <section id="installers" class="section section--tint anchor">
      <div class="container">
        <span class="eyebrow">{{ $t('download:your_own_hardware') }}</span>
        <h2 class="section-title">{{ $t('download:installers_bia_deploy') }}</h2>
        <p class="section-lead">
          {{ $t('download:free_and_open_source_bia_deploy') }}
        </p>
        <h3 class="h3">{{ $t('download:on_a_computer') }}</h3>
        <p class="h3note">{{ $t('download:servers_and_usb_sticks') }}</p>
        <div class="grid grid--3 hosts">
          <div v-for="host in hosts" :key="host.id" class="card host">
            <h3>{{ host.label }}</h3>
            <p class="host__note">{{ host.note }}</p>
            <a
              class="btn btn--primary host__btn"
              :class="{ 'is-busy': loading }"
              :href="downloads[host.id].url"
              :aria-busy="loading"
            >
              {{ buttonText(host) }}
            </a>
            <p v-if="downloads[host.id].available" class="host__ver">{{ $t('download:version', { version: downloads[host.id].version }) }}</p>
          </div>
        </div>
        <h3 class="h3">{{ $t('download:on_a_phone') }}</h3>
        <p class="h3note">{{ $t('download:servers_only_the_same_app_is') }}</p>
        <div class="grid grid--3 hosts phones">
          <div class="card host">
            <h3>Android</h3>
            <p class="host__note">{{ $t('download:phones_and_tablets') }}</p>
            <a class="btn btn--primary host__btn" :href="PLAY_URL" target="_blank" rel="noopener">{{ $t('download:get_it_on_google_play') }}</a>
            <a
              class="btn btn--ghost host__btn host__btn--alt"
              :class="{ 'is-busy': loading }"
              :href="appDownloads.android.url"
              :aria-busy="loading"
            >
              {{ $t('download:download_the_apk') }}
            </a>
            <p class="host__ver">
              <template v-if="appDownloads.android.available">
                {{ $t('download:version', { version: appDownloads.android.version }) }}<template v-if="appDownloads.android.size"> · {{ sizeMB(appDownloads.android.size) }}</template> ·
              </template>
              {{ $t('download:from_github_no_play_store_needed') }}
            </p>
          </div>
          <div class="card host">
            <h3>iPhone</h3>
            <p class="host__note">{{ $t('download:iphone_and_ipad') }}</p>
            <a class="btn btn--primary host__btn" :href="APP_STORE_URL" target="_blank" rel="noopener">{{ $t('download:download_on_the_app_store') }}</a>
            <a class="btn btn--ghost host__btn host__btn--alt" :href="TESTFLIGHT_URL" target="_blank" rel="noopener">{{ $t('download:join_the_testflight_beta') }}</a>
            <a class="btn btn--ghost host__btn host__btn--alt" :href="APPDB_URL" target="_blank" rel="noopener">{{ $t('download:get_it_on_appdb') }}</a>
            <p class="host__ver">{{ $t('download:app_store_testflight_for_betas_appdb') }}</p>
          </div>
          <div class="card host">
            <h3>{{ $t('download:other_ways_in') }}</h3>
            <p class="host__note">{{ $t('download:for_the_technical') }}</p>
            <ul class="ways">
              <li><Tr k="download:one_command_on_a_running_linux"><strong /><NuxtLink :to="$lp('/servers')" /></Tr></li>
              <li><Tr k="download:on_a_mac_in_a_virtual"><strong /><a href="https://github.com/ZonD80/clawhome/releases" target="_blank" rel="noopener" /></Tr></li>
              <li><Tr k="download:from_source_gpl_3_0_github"><strong /><a :href="REPO_URL" target="_blank" rel="noopener" /></Tr></li>
            </ul>
          </div>
        </div>
        <p v-if="version" class="release-note">
          <Tr k="download:latest_bia_version_release_notes" :v="{ version }"><a :href="releaseUrl" /></Tr>
        </p>
        <p v-if="error" class="release-note release-note--warn">
          {{ error }} — <a :href="fallbackUrl">{{ $t('download:open_the_releases_page') }}</a>
        </p>
      </div>
    </section>

    <!-- 3. how a bia lives -->
    <section id="life" class="section anchor">
      <div class="container">
        <span class="eyebrow">{{ $t('download:how_a_bia_lives') }}</span>
        <h2 class="section-title">{{ $t('download:where_it_can_live_and_what') }}</h2>
        <p class="section-lead">
          {{ $t('download:the_computer_is_bias_body_not') }}
        </p>

        <h3 class="h3">{{ $t('download:where_it_can_live') }}</h3>
        <div class="grid grid--feat homes">
          <div v-for="h in homes" :key="h.title" class="card home">
            <AppIcon :name="h.icon" size="1.8rem" class="ico" />
            <h4>{{ h.title }}</h4>
            <p>{{ h.text }}</p>
          </div>
        </div>

        <h3 class="h3">{{ $t('download:what_it_does_there') }}</h3>
        <div class="grid grid--2 lives">
          <div v-for="l in lives" :key="l.title" class="card life">
            <AppIcon :name="l.icon" size="2rem" class="ico" />
            <div>
              <h4>{{ l.title }}</h4>
              <p>{{ l.text }}</p>
              <NuxtLink v-if="l.to" :to="$lp(l.to)">{{ l.more }}</NuxtLink>
            </div>
          </div>
        </div>
      </div>
    </section>

    <!-- 4. the stick, step by step -->
    <section id="stick" class="section section--tint anchor">
      <div class="container">
        <span class="eyebrow">{{ $t('download:step_by_step') }}</span>
        <h2 class="section-title">{{ $t('download:a_computer_you_already_have') }}</h2>
        <p class="section-lead">
          {{ $t('download:about_twenty_minutes_download_bia') }}
        </p>
        <h3 class="h3">{{ $t('download:choose_how_bia_should_think') }}</h3>
        <div class="grid grid--2">
          <div class="card mode">
            <AppIcon name="globe" size="2rem" class="ico" />
            <h3>{{ $t('download:cloud_brain_recommended') }}</h3>
            <p>
              {{ $t('download:the_smartest_answers_using_an_ai') }}
            </p>
          </div>
          <div class="card mode">
            <AppIcon name="brain" size="2rem" class="ico" />
            <h3>{{ $t('download:brain_on_the_stick_optional') }}</h3>
            <p>
              {{ $t('download:for_places_without_internet_a_complete') }}
            </p>
          </div>
        </div>

        <h3 class="h3">{{ $t('download:write_the_stick_and_start_a') }}</h3>
        <ol class="final list-check">
          <li>{{ $t('download:plug_in_a_usb_stick_of') }}</li>
          <li>{{ $t('download:open_bia_deploy_and_choose_the') }}</li>
          <li>{{ $t('download:choose_on_a_usb_stick_pick') }}</li>
          <li>{{ $t('download:plug_the_stick_into_the_computer') }}</li>
          <li>{{ $t('download:bia_wakes_up_and_greets_you') }}</li>
        </ol>
        <div class="note-box tip kinds">
          <p>
            <Tr k="download:one_stick_one_kind_of_computer"><strong /></Tr>
          </p>
          <ul>
            <li>
              <Tr k="download:ordinary_pc_almost_any_intel_or"><strong /></Tr>
            </li>
            <li>
              <Tr k="download:arm_computer_arm_mini_pcs_and"><strong /></Tr>
            </li>
            <li>
              <Tr k="download:not_supported_macs_with_apple_chips"><strong /></Tr>
            </li>
          </ul>
          <p>
            <Tr k="download:bia_already_lives_on_another_kind"><strong /><NuxtLink :to="$lp('/features/moving')" /></Tr>
          </p>
        </div>
        <div class="note-box tip kinds">
          <p>
            <Tr k="download:a_server_instead_of_a_stick"><strong /><NuxtLink :to="$lp('/servers#takeover')" /></Tr>
          </p>
        </div>
      </div>
    </section>

    <CtaBand :title="$t('download:next_introduce_it')" :text="$t('download:tell_it_its_name_give_it')" />
  </main>
</template>

<script setup lang="ts">
const t = useT()
useSeoMeta({
  title: t('download:get_bia_in_the_cloud_on'),
  description: t('download:where_a_bia_can_live_and'),
})

import type { FlashHost } from '~/composables/useLatestRelease'


type Host = { id: FlashHost; label: string; note: string }

// the phone apps (server only)
const PLAY_URL = 'https://play.google.com/store/apps/details?id=biadeploy.android'
const APP_STORE_URL = 'https://apps.apple.com/app/bia-deploy/id6827948196'
const TESTFLIGHT_URL = 'https://testflight.apple.com/join/qePtQWqF'
const APPDB_URL = 'https://appdb.to/details/c25d9b6ff3034e039c294b1acd92eafcfc14c9b0'

const hosts: Host[] = [
  { id: 'macos', label: t('download:mac'), note: t('download:apple_silicon_and_intel') },
  { id: 'windows', label: t('download:windows'), note: t('download:n64_bit') },
  { id: 'linux', label: t('download:linux'), note: t('download:64_bit_intel_and_amd') },
]

const { downloads, appDownloads, version, releaseUrl, loading, error, fetched, fallbackUrl } = useLatestRelease()

const sizeMB = (bytes: number) => `${Math.round(bytes / 1e6)} MB`

function buttonText(host: Host): string {
  if (loading.value) return t('common:checking_latest')
  if (fetched.value && !downloads.value[host.id].available) return t('common:see_releases')
  return t('download:download_for', { label: host.label })
}

const homes = [
  { icon: 'globe', title: t('download:bia_cloud'), text: t('download:a_server_we_keep_running_and') },
  { icon: 'link', title: t('download:a_server_you_rent'), text: t('download:any_linux_server_or_vps_bia') },
  { icon: 'terminal', title: t('download:an_old_pc_or_laptop'), text: t('download:almost_any_intel_or_amd_machine') },
  { icon: 'stick', title: t('download:a_usb_stick'), text: t('download:bia_can_stay_on_the_stick') },
  { icon: 'window', title: t('download:a_virtual_machine'), text: t('download:on_a_mac_with_clawhome_or') },
]

const lives = [
  { icon: 'move', title: t('download:it_moves'), text: t('download:bia_folds_itself_into_one_archive'), to: '/features/moving', more: t('download:how_moving_works') },
  { icon: 'refresh', title: t('download:it_updates_and_repairs_itself'), text: t('download:it_installs_its_own_system_updates'), to: '/features/updates', more: t('download:how_updates_work') },
  { icon: 'clock', title: t('download:it_acts_on_its_own'), text: t('download:bia_has_a_clock_and_a'), to: '/features/calendar', more: t('download:time_and_reminders') },
  { icon: 'puzzle', title: t('download:it_develops_itself'), text: t('download:when_a_tool_is_missing_bia'), to: '/features/develops-itself', more: t('download:how_it_grows') },
  { icon: 'people', title: t('download:it_sets_up_new_bias'), text: t('download:give_a_bia_the_login_of'), to: '/servers', more: t('download:bias_and_servers') },
  { icon: 'brain', title: t('download:it_chooses_how_to_think'), text: t('download:its_brain_is_an_ai_model'), to: '/features/brain', more: t('download:about_the_brain') },
  { icon: 'key', title: t('download:it_keeps_its_own_keys'), text: t('download:passwords_one_time_codes_and_passkeys'), to: '/features/vault', more: t('download:the_safe') },
  { icon: 'shield', title: t('download:it_stays_separate'), text: t('download:bia_never_asks_for_the_keys'), to: '/features/privacy', more: t('download:privacy') },
]
</script>

<style scoped>
.anchor { scroll-margin-top: calc(4.5rem + var(--strip-h, 0px)); }
.jump { display: flex; flex-wrap: wrap; gap: 0.45rem; margin-top: 1.5rem; }
.h3 { font-size: clamp(1.25rem, 2.6vw, 1.55rem); margin: 2.25rem 0 0.25rem; }
.h3 + .grid, .h3 + ol { margin-top: 1rem; }
.h3note { color: var(--mute); margin: 0 0 1rem; }
.cloud { display: grid; gap: 2rem; grid-template-columns: 1fr minmax(8rem, 14rem); align-items: center; padding: clamp(1.4rem, 3vw, 2.25rem); border-color: rgba(63, 111, 42, 0.3); }
.cloud h2 { font-size: clamp(1.6rem, 3.5vw, 2.2rem); margin: 0.2rem 0 0.6rem; }
.cloud .list-check { margin: 1rem 0 1.5rem; }
.cloud__art { animation: float 8s ease-in-out infinite; }
.ways { margin: 0; padding-left: 1.1rem; color: var(--ink-soft); font-size: 0.95rem; }
.ways li + li { margin-top: 0.5rem; }
.homes { grid-template-columns: repeat(auto-fit, minmax(min(100%, 12.5rem), 1fr)); }
.home h4, .life h4 { font-size: 1.1rem; margin: 0 0 0.3rem; font-family: var(--font-display); }
.home p { margin: 0; color: var(--ink-soft); font-size: 0.94rem; }
.life { display: flex; gap: 1rem; align-items: flex-start; }
.life .ico { flex-shrink: 0; margin: 0.15rem 0 0; }
.life p { margin: 0 0 0.4rem; color: var(--ink-soft); }
.life a { font-weight: 700; font-size: 0.92rem; }
@media (max-width: 700px) {
  .cloud { grid-template-columns: 1fr; }
  .cloud__art { display: none; }
}
.host h3 { font-size: 1.4rem; margin-bottom: 0.2rem; }
.host__note { color: var(--mute); }
.host__btn { width: 100%; }
.host__btn--alt { margin-top: 0.6rem; }
.host__btn.is-busy { opacity: 0.7; pointer-events: none; }
.host__ver { margin: 0.5rem 0 0; font-size: 0.85rem; color: var(--mute); text-align: center; }
.release-note { margin: 1rem 0 0; font-size: 0.9rem; color: var(--mute); }
.release-note--warn { color: #9a6a12; }
.ico { color: var(--accent); margin-bottom: 0.6rem; }
.final { margin-bottom: 1.5rem; }
.tip { margin-top: 1rem; }
.kinds p { margin: 0; }
.kinds ul { margin: 0.75rem 0; padding-left: 1.2rem; }
.kinds li + li { margin-top: 0.4rem; }
</style>
