<template>
  <main>
    <section class="page-hero">
      <div class="container">
        <span class="eyebrow">{{ $t('open:a_link_from_your_bia') }}</span>
        <h1>{{ $t('open:open_it_in_bia_deploy') }}</h1>
        <p class="section-lead">
          {{ $t('open:your_bia_sent_a_page_that') }}
        </p>
      </div>
    </section>

    <section class="section">
      <div class="container narrow">
        <p v-if="!link" class="card warn">
          {{ $t('open:this_link_is_not_complete_ask') }}
        </p>
        <template v-else>
          <a class="btn btn--primary wide" :href="appUrl">{{ $t('open:open_in_bia_deploy') }}</a>
          <p class="mute small">{{ $t('open:opens_on_bia', { what }) }}<template v-if="link.at.length">{{ $t('open:at', { at: link.at[0] }) }}</template>.</p>

          <h2 class="h2">{{ $t('open:no_app_yet') }}</h2>
          <div class="grid grid--2">
            <div class="stores">
              <h3 class="h3">Android</h3>
              <a class="btn btn--ghost wide" :href="PLAY_URL" target="_blank" rel="noopener">{{ $t('open:get_it_on_google_play') }}</a>
              <a class="btn btn--ghost wide" :href="appDownloads.android.url" :aria-busy="loading">{{ $t('open:download_the_apk') }}</a>
              <p class="mute small">
                <template v-if="appDownloads.android.available">{{ $t('open:version', { version: appDownloads.android.version }) }}</template>{{ $t('open:apk_from_github_no_play_store') }}
              </p>
            </div>
            <div class="stores">
              <h3 class="h3">iPhone</h3>
              <a class="btn btn--ghost wide" :href="APP_STORE_URL" target="_blank" rel="noopener">{{ $t('open:download_on_the_app_store') }}</a>
              <a class="btn btn--ghost wide" :href="TESTFLIGHT_URL" target="_blank" rel="noopener">{{ $t('open:join_the_testflight_beta') }}</a>
              <a class="btn btn--ghost wide" :href="APPDB_URL" target="_blank" rel="noopener">{{ $t('open:get_it_on_appdb') }}</a>
            </div>
          </div>
          <p class="mute small"><Tr k="open:all_downloads_including_bia_deploy_for"><NuxtLink :to="$lp('/download')" /></Tr></p>
          <p class="mute small">
            {{ $t('open:then_come_back_to_this_message') }}
          </p>

          <template v-if="direct.length">
            <h2 class="h2">{{ $t('open:on_a_computer_or_a_phone') }}</h2>
            <p>{{ $t('open:the_same_page_in_a_browser') }}</p>
            <ul class="direct">
              <li v-for="u in direct" :key="u"><a :href="u" rel="noopener">{{ u }}</a></li>
            </ul>
          </template>
        </template>
      </div>
    </section>
  </main>
</template>

<script setup lang="ts">
const t = useT()
useSeoMeta({
  title: t('open:open_in_bia_deploy'),
  description: t('open:a_link_from_your_bia_that'),
  robots: 'noindex, nofollow',
})

const PLAY_URL = 'https://play.google.com/store/apps/details?id=biadeploy.android'
const APP_STORE_URL = 'https://apps.apple.com/app/bia-deploy/id6827948196'
const TESTFLIGHT_URL = 'https://testflight.apple.com/join/qePtQWqF'
const APPDB_URL = 'https://appdb.to/details/c25d9b6ff3034e039c294b1acd92eafcfc14c9b0'

const { appDownloads, loading } = useLatestRelease()

type Link = { ca: string; at: string[]; go: string; query: string }

// The link lives in the fragment (#ca=…&at=…&go=…), which never reaches this site's server.
// The router's first navigation drops the fragment in the static build before onMounted runs,
// so an inline script in the head keeps a copy as soon as the HTML loads.
useHead({ script: [{ key: 'bia-link-hash', innerHTML: 'window.__biaLinkHash=location.hash' }] })
const link = ref<Link | null>(null)
const android = ref(false)

onMounted(() => {
  android.value = /Android/i.test(navigator.userAgent)
  const raw = (location.hash || (window as any).__biaLinkHash || '').replace(/^#/, '')
  const p = new URLSearchParams(raw)
  const ca = p.get('ca') || ''
  const go = p.get('go') || '/'
  if (!/^[0-9a-f]{32}$/i.test(ca) || !go.startsWith('/')) return
  const at = (p.get('at') || '').split(',').map((s) => s.trim()).filter((s) => /^[\w.:\[\]-]+$/.test(s))
  link.value = { ca, at, go, query: raw }
})

// A custom scheme takes over when the universal link / App Link did not: query, not fragment,
// since Android's intent: links use the fragment themselves.
const appUrl = computed(() => {
  if (!link.value) return '#'
  const q = link.value.query
  if (android.value) {
    return `intent://open?${q}#Intent;scheme=biadeploy;package=biadeploy.android;S.browser_fallback_url=${encodeURIComponent(PLAY_URL)};end`
  }
  return `biadeploy://open?${q}`
})

const what = computed(() => {
  const go = link.value?.go || '/'
  if (go.startsWith('/hitl/')) return t('open:what_task')
  if (go.startsWith('/dl/')) return t('open:what_file')
  return t('open:what_chat')
})

const direct = computed(() => (link.value ? link.value.at.map((h) => `https://${h}${link.value!.go}`) : []))
</script>

<style scoped>
.narrow { max-width: 40rem; }
.wide { width: 100%; }
.h2 { font-size: clamp(1.2rem, 2.6vw, 1.5rem); margin: 2.25rem 0 0.75rem; }
.mute { color: var(--mute); }
.small { font-size: 0.9rem; margin-top: 0.6rem; }
.warn { color: #9a6a12; }
.stores { display: flex; flex-direction: column; gap: 0.6rem; }
.stores .small { margin-top: 0; }
.h3 { font-size: 1.05rem; margin: 0; }
.direct { padding-left: 1.2rem; }
.direct a { overflow-wrap: anywhere; }
</style>
