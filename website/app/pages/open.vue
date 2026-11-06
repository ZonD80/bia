<template>
  <main>
    <section class="page-hero">
      <div class="container">
        <span class="eyebrow">A link from your bia</span>
        <h1>Open it in bia deploy</h1>
        <p class="section-lead">
          Your bia sent a page that opens in the bia deploy app, which holds bia's key on your phone. If you see
          this, the app did not open by itself: open it with the button, or get the app first.
        </p>
      </div>
    </section>

    <section class="section">
      <div class="container narrow">
        <p v-if="!link" class="card warn">
          This link is not complete. Ask your bia to send it again.
        </p>
        <template v-else>
          <a class="btn btn--primary wide" :href="appUrl">Open in bia deploy</a>
          <p class="mute small">Opens {{ what }} on bia<template v-if="link.at.length"> at {{ link.at[0] }}</template>.</p>

          <h2 class="h2">No app yet?</h2>
          <div class="grid grid--2">
            <div class="stores">
              <h3 class="h3">Android</h3>
              <a class="btn btn--ghost wide" :href="PLAY_URL" target="_blank" rel="noopener">Get it on Google Play</a>
              <a class="btn btn--ghost wide" :href="appDownloads.android.url" :aria-busy="loading">Download the APK</a>
              <p class="mute small">
                <template v-if="appDownloads.android.available">Version {{ appDownloads.android.version }} · </template>APK from GitHub, no Play Store needed
              </p>
            </div>
            <div class="stores">
              <h3 class="h3">iPhone</h3>
              <a class="btn btn--ghost wide" :href="APP_STORE_URL" target="_blank" rel="noopener">Download on the App Store</a>
              <a class="btn btn--ghost wide" :href="TESTFLIGHT_URL" target="_blank" rel="noopener">Join the TestFlight beta</a>
              <a class="btn btn--ghost wide" :href="APPDB_URL" target="_blank" rel="noopener">Get it on appdb</a>
            </div>
          </div>
          <p class="mute small">All downloads, including bia deploy for Mac, Windows and Linux: <NuxtLink to="/download">Get bia</NuxtLink>.</p>
          <p class="mute small">
            Then come back to this message and tap the link again. The link brings bia's key to the app, so nothing else
            is needed. That key works once and only for 10 minutes; after that, ask bia for a new link.
          </p>

          <template v-if="direct.length">
            <h2 class="h2">On a computer</h2>
            <p>The same page in a browser that has bia's certificate:</p>
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
useSeoMeta({
  title: 'Open in bia deploy',
  description: 'A link from your bia that opens in the bia deploy phone app.',
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
  if (go.startsWith('/hitl/')) return 'a task for you'
  if (go.startsWith('/dl/')) return 'a file'
  return 'the chat'
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
