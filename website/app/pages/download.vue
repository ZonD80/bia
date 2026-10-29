<template>
  <main>
    <section class="page-hero">
      <div class="container">
        <nav class="crumbs" aria-label="Breadcrumb">
          <NuxtLink to="/">Home</NuxtLink><span aria-hidden="true">/</span><span>Get bia</span>
        </nav>
        <span class="eyebrow">Free software</span>
        <h1>Wake your bia</h1>
        <p class="section-lead">
          Download bia deploy for your computer. It writes bia onto a USB stick, then you start any PC or laptop
          from it. It can also put bia on a server you rent, with no commands to type, and so can its phone app.
        </p>
      </div>
    </section>

    <section class="section">
      <div class="container">
        <h2 class="h2">1. Download bia deploy</h2>
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
            <p v-if="downloads[host.id].available" class="host__ver">Version {{ downloads[host.id].version }}</p>
          </div>
        </div>
        <h3 class="phones__title">Or from your phone</h3>
        <p class="phones__lead">The phone app puts bia on a server you rent (no stick from a phone).</p>
        <div class="grid grid--3 hosts phones">
          <div class="card host">
            <h3>Android</h3>
            <p class="host__note">Phones and tablets</p>
            <a class="btn btn--primary host__btn" :href="PLAY_URL" target="_blank" rel="noopener">Get it on Google Play</a>
            <a
              class="btn btn--ghost host__btn host__btn--alt"
              :class="{ 'is-busy': loading }"
              :href="appDownloads.android.url"
              :aria-busy="loading"
            >
              Download the APK
            </a>
            <p class="host__ver">
              <template v-if="appDownloads.android.available">
                Version {{ appDownloads.android.version }}<template v-if="appDownloads.android.size"> · {{ sizeMB(appDownloads.android.size) }}</template> ·
              </template>
              from GitHub, no Play Store needed
            </p>
          </div>
          <div class="card host host--soon">
            <h3>iPhone</h3>
            <p class="host__note">iPhone and iPad</p>
            <span class="btn btn--ghost host__btn" aria-disabled="true">Coming soon to the App Store</span>
          </div>
        </div>
        <p v-if="version" class="release-note">
          Latest bia: version {{ version }} · <a :href="releaseUrl">release notes</a>
        </p>
        <p v-if="error" class="release-note release-note--warn">
          {{ error }} — <a :href="fallbackUrl">open the releases page</a>
        </p>

        <h2 class="h2">2. Choose how bia should think</h2>
        <div class="grid grid--2">
          <div class="card mode">
            <AppIcon name="globe" size="2rem" class="ico" />
            <h3>Cloud brain (recommended)</h3>
            <p>
              The smartest answers, using an AI service with your own account. Right now the cheapest good choice
              is DeepSeek V4.1: you pay only for what bia uses, and it runs on any computer. The stick stays small,
              but bia needs the internet. Paste your key into bia deploy and it is stored only on your stick.
            </p>
          </div>
          <div class="card mode">
            <AppIcon name="brain" size="2rem" class="ico" />
            <h3>Brain on the stick (optional)</h3>
            <p>
              For places without internet, a complete AI model can travel on the stick instead. Needs a larger
              stick and a powerful computer: a high-end graphics card, or unified memory, with at least 64 GB of
              video memory (VRAM). You can switch between the two later.
            </p>
          </div>
        </div>

        <h2 class="h2">3. Write the stick and start a computer</h2>
        <ol class="final list-check">
          <li>Plug in a USB stick of 8 GB or more. Everything on it will be replaced.</li>
          <li>Open bia deploy and choose the brain: sign in with ChatGPT or paste a key, and press Wake bia up. bia says hello when the brain works.</li>
          <li>Choose “On a USB stick”, pick the stick and the kind of computer it is for, and press Write. The first time, it downloads the bia system for that kind of computer (about 1 GB) and keeps it for the next stick.</li>
          <li>Plug the stick into the computer that will become bia and switch it on. If it starts its old system instead, open the computer’s start-up menu (often F12, F11, Esc or Option ⌥) and choose the USB stick.</li>
          <li>bia wakes up and greets you. Say hello!</li>
        </ol>
        <div class="note-box tip kinds">
          <p>
            <strong>One stick, one kind of computer.</strong> Ordinary PCs and ARM computers need different
            systems, so bia deploy asks which kind the stick is for. Have both? Make one stick for each.
          </p>
          <ul>
            <li>
              <strong>Ordinary PC:</strong> almost any Intel or AMD desktop or laptop, such as a ten-year-old office
              PC, a gaming PC or a Windows laptop. It starts with Secure Boot switched on, and older machines
              without the modern start-up (UEFI) start it the classic way (BIOS).
            </li>
            <li>
              <strong>ARM computer:</strong> ARM mini-PCs and boards whose start-up menu works the standard
              (UEFI) way. Whether Secure Boot can stay on depends on the machine: if the stick does not start,
              switch Secure Boot off in its settings.
            </li>
            <li>
              <strong>Not supported:</strong> Macs with Apple chips (M1 and newer) cannot start from a bia
              stick. They can still run bia deploy.
            </li>
          </ul>
          <p>
            <strong>bia already lives on another kind of computer?</strong> Its mind can move across. For
            example, bia runs on an old Intel laptop and you bought an ARM mini-PC: make an ARM stick, start the
            mini-PC from it and tell the new bia to wait for the old one. Then ask the old bia to move over. Both
            screens show the same code, and once you confirm, its chats, memories, accounts and passwords arrive
            on the mini-PC. Keep the mini-PC online during the move, so bia can reinstall its extra programs in
            their ARM versions. <NuxtLink to="/features/moving">How moving works</NuxtLink>
          </p>
        </div>
        <div class="note-box tip kinds">
          <p>
            <strong>A server instead of a stick?</strong> Choose “On my server” after the brain. bia deploy logs in
            to the server with its password or key, checks it, and replaces its system with bia.
            <NuxtLink to="/servers#takeover">More about servers</NuxtLink>
          </p>
        </div>
      </div>
    </section>

    <section id="other-ways" class="section section--tint other-ways">
      <div class="container">
        <h2 class="h2">Other ways</h2>
        <div class="grid grid--3">
          <div class="card">
            <h3>Try it on your Mac</h3>
            <p>No stick needed: <a href="https://github.com/ZonD80/clawhome" target="_blank" rel="noopener">ClawHome</a> runs bia in a virtual machine on an Apple Silicon Mac (macOS 13 or later). Create a Linux home, sign in to a cloud AI or paste a key, and ClawHome fetches the latest bia for you.</p>
            <p><a href="https://github.com/ZonD80/clawhome/releases" target="_blank" rel="noopener">Get ClawHome</a></p>
          </div>
          <div class="card">
            <h3>Servers and virtual machines</h3>
            <p>A rented server or any machine running Linux becomes bia with one command over ssh: <code>curl -fsSL https://github.com/zond80/bia/releases/latest/download/takeover.sh | sudo bash</code>. It can start as a new bia, or wait for yours to fly there. Your bia can even set a server up by itself. Empty machines start from the installer image.</p>
            <p><NuxtLink to="/servers">Read the server guide</NuxtLink></p>
          </div>
          <div class="card">
            <h3>For developers</h3>
            <p>bia is open source under GPL-3.0. The whole system, including bia deploy, can be built from source on Mac or Linux with Docker.</p>
          </div>
        </div>
      </div>
    </section>

    <CtaBand title="Next: meet it" text="Not sure what to ask first? Look through the use cases for ideas." />
  </main>
</template>

<script setup lang="ts">
useSeoMeta({
  title: 'Wake your bia — on a USB stick or your server',
  description: 'Download bia deploy for Mac, Windows, Linux or Android: write bia onto a USB stick or put it on your server, with a cloud brain or a brain on the stick.',
})

import type { FlashHost } from '~/composables/useLatestRelease'

type Host = { id: FlashHost; label: string; note: string }

// the phone apps (server only); the iPhone one is not in the App Store yet
const PLAY_URL = 'https://play.google.com/store/apps/details?id=biadeploy.android'

const hosts: Host[] = [
  { id: 'macos', label: 'Mac', note: 'Apple Silicon and Intel' },
  { id: 'windows', label: 'Windows', note: '64-bit' },
  { id: 'linux', label: 'Linux', note: '64-bit (Intel and AMD)' },
]

const { downloads, appDownloads, version, releaseUrl, loading, error, fetched, fallbackUrl } = useLatestRelease()

const sizeMB = (bytes: number) => `${Math.round(bytes / 1e6)} MB`

function buttonText(host: Host): string {
  if (loading.value) return 'Checking latest…'
  if (fetched.value && !downloads.value[host.id].available) return 'See releases'
  return `Download for ${host.label}`
}
</script>

<style scoped>
.h2 {
  font-size: clamp(1.4rem, 3vw, 1.9rem);
  margin: 2.5rem 0 1rem;
}
.h2:first-child { margin-top: 0; }
.host h3 { font-size: 1.4rem; margin-bottom: 0.2rem; }
.host__note { color: var(--mute); }
.host__btn { width: 100%; }
.host__btn--alt { margin-top: 0.6rem; }
.host__btn.is-busy { opacity: 0.7; pointer-events: none; }
.phones__title { font-size: 1.2rem; margin: 2rem 0 0.25rem; }
.phones__lead { color: var(--mute); margin: 0 0 1rem; }
.host--soon .host__btn { cursor: default; opacity: 0.7; }
.host__ver { margin: 0.5rem 0 0; font-size: 0.85rem; color: var(--mute); text-align: center; }
.release-note { margin: 1rem 0 0; font-size: 0.9rem; color: var(--mute); }
.release-note--warn { color: #9a6a12; }
.ico { color: var(--accent); margin-bottom: 0.6rem; }
.final { margin-bottom: 1.5rem; }
.tip { margin-top: 1rem; }
.kinds p { margin: 0; }
.kinds ul { margin: 0.75rem 0; padding-left: 1.2rem; }
.kinds li + li { margin-top: 0.4rem; }
.other-ways { scroll-margin-top: 4.5rem; }
.other-ways code { overflow-wrap: anywhere; font-size: 0.85em; }
</style>
