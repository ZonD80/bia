<template>
  <main>
    <section class="page-hero">
      <div class="container">
        <nav class="crumbs" aria-label="Breadcrumb">
          <NuxtLink to="/">Home</NuxtLink><span aria-hidden="true">/</span><span>Get bia</span>
        </nav>
        <span class="eyebrow">Free software</span>
        <h1>Get bia onto a USB stick</h1>
        <p class="section-lead">
          Download the bia writer for your computer, plug in a USB stick, and follow the three screens. Then start
          any PC or laptop from the stick.
        </p>
      </div>
    </section>

    <section class="section">
      <div class="container">
        <h2 class="h2">1. Download the writer</h2>
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
              but bia needs the internet. Paste your key into the writer and it is stored only on your stick.
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
          <li>Open the writer, pick the stick and the kind of computer it is for, choose the brain, and press Write. The first time, it downloads the bia system for that kind of computer (about 1 GB) and keeps it for the next stick.</li>
          <li>Plug the stick into the computer that will become bia and switch it on. If it starts its old system instead, open the computer’s start-up menu (often F12, F11, Esc or Option ⌥) and choose the USB stick.</li>
          <li>bia wakes up and greets you. Say hello!</li>
        </ol>
        <div class="note-box tip kinds">
          <p>
            <strong>One stick, one kind of computer.</strong> Ordinary PCs and ARM computers need different
            systems, so the writer asks which kind the stick is for. Have both? Make one stick for each.
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
              stick. They can still run the writer.
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
            <p>bia is open source under GPL-3.0. The whole system, including the stick writer, can be built from source on Mac or Linux with Docker.</p>
          </div>
        </div>
      </div>
    </section>

    <CtaBand title="Next: meet it" text="Not sure what to ask first? Look through the use cases for ideas." />
  </main>
</template>

<script setup lang="ts">
useSeoMeta({
  title: 'Get bia — write a USB stick',
  description: 'Download the bia writer for Mac, Windows or Linux, choose a cloud brain or a brain on the stick, and wake up any computer.',
})

import type { FlashHost } from '~/composables/useLatestRelease'

type Host = { id: FlashHost; label: string; note: string }

const hosts: Host[] = [
  { id: 'macos', label: 'Mac', note: 'Apple Silicon and Intel' },
  { id: 'windows', label: 'Windows', note: '64-bit' },
  { id: 'linux', label: 'Linux', note: '64-bit (Intel and AMD)' },
]

const { downloads, version, releaseUrl, loading, error, fetched, fallbackUrl } = useLatestRelease()

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
.other-ways { scroll-margin-top: 4.5rem; }
.other-ways code { overflow-wrap: anywhere; font-size: 0.85em; }
</style>
