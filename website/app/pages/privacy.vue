<template>
  <main>
    <section class="page-hero">
      <div class="container">
        <nav class="crumbs" aria-label="Breadcrumb">
          <NuxtLink to="/">Home</NuxtLink><span aria-hidden="true">/</span><span>Privacy</span>
        </nav>
        <span class="eyebrow">Privacy policy</span>
        <h1>We collect no data</h1>
        <p class="section-lead">
          Not on this website, not in the bia deploy apps, not in bia. There are no accounts, no analytics, no ads and
          no telemetry, and nothing is sent to us. Updated {{ UPDATED }}.
        </p>
      </div>
    </section>

    <section class="section">
      <div class="narrow policy">
        <div v-for="s in sections" :key="s.title">
          <h2>{{ s.title }}</h2>
          <p v-for="(p, i) in s.text" :key="i">{{ p }}</p>
        </div>
        <h2>Questions</h2>
        <p>
          Ask in the project's issues on GitHub:
          <a :href="`${REPO_URL}/issues`" target="_blank" rel="noopener">{{ REPO_URL.replace('https://', '') }}/issues</a>.
          The source code of everything described here is public, so every claim on this page can be checked.
        </p>
      </div>
    </section>
  </main>
</template>

<script setup lang="ts">
import { REPO_URL } from '~/composables/useLatestRelease'

const UPDATED = '28 October 2026'

useSeoMeta({
  title: 'Privacy policy — bia',
  description: 'bia, the bia deploy apps (Mac, Windows, Linux, Android, iPhone) and this website collect no data: no accounts, analytics, ads or telemetry.',
})

const sections = [
  {
    title: 'This website',
    text: [
      'ownbia.com sets no cookies, runs no analytics or tracking scripts and has no forms or accounts.',
      'Like any website, it is delivered by its host (Cloudflare Pages), which handles each request, including your IP address, to serve the page. Its fonts come from Google Fonts, so your browser also asks Google for them. Fred’s film plays from YouTube (youtube-nocookie.com) only after you press play; until then nothing is loaded from YouTube.',
    ],
  },
  {
    title: 'bia deploy (Mac, Windows, Linux, Android and iPhone)',
    text: [
      'The app needs no account and sends nothing to us. What you enter stays on your device or goes straight to the service it is meant for:',
      'Your AI key goes to the AI provider you chose, to check it and to wake bia once. A ChatGPT sign-in goes to OpenAI. The bia system is downloaded from GitHub, and a model, if you pick one, from Hugging Face. A server’s address, user and password or SSH key are used to log in to that server over SSH; the password is used only then and is not saved.',
      'An SSH key the app makes, the servers it has seen, and each bia’s key (its certificate) are kept in the app’s own storage on your device, so you can use them again. Uninstalling the app removes them. Sharing a bia’s key happens only when you press Share, to the app you choose.',
      'The app does not save your AI key. After setup, the key and a ChatGPT sign-in belong to your bia, and the app drops its copy of the sign-in, keeping only its own app registration with OpenAI.',
    ],
  },
  {
    title: 'bia itself',
    text: [
      'bia runs on a computer or server you control, and its memory, files, contacts and passwords stay there. We never receive any of it.',
      'To work, bia talks to the services you set it up with: the AI provider you chose receives what bia thinks about (that is how a cloud brain works; a brain on the stick keeps even that local), and the messengers you connect carry its messages. It downloads updates, languages and voices from GitHub and Hugging Face. Each of those services follows its own privacy policy.',
    ],
  },
  {
    title: 'Changes',
    text: ['If this ever changes, this page will say so first, with a new date.'],
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
