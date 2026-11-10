<template>
  <main>
    <section class="page-hero">
      <div class="container">
        <nav class="crumbs" aria-label="Breadcrumb">
          <NuxtLink to="/">Home</NuxtLink><span aria-hidden="true">/</span><span>bia cloud</span>
        </nav>
        <span class="eyebrow">bia cloud · early access</span>
        <h1>No computer to spare? Let us run one for it.</h1>
        <p class="section-lead">
          bia cloud gives a bia a server of its own, keeps it running and backed up, and lets you pay as you go.
          It is the same free bia — just already awake, on its own computer in a European data centre.
        </p>
        <div class="btn-row">
          <a class="btn btn--primary" :href="`${CLOUD}/#start`">Wake up a bia in the cloud</a>
          <a class="btn btn--ghost" :href="`${CLOUD}/pricing/`">Prices</a>
        </div>
      </div>
    </section>

    <section class="section">
      <div class="container">
        <span class="eyebrow">How it works</span>
        <h2 class="section-title">A name, a language, one payment</h2>
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
        <span class="eyebrow">Pay as you go</span>
        <h2 class="section-title">One balance, no plan</h2>
        <div class="grid grid--3">
          <div class="card">
            <h3>Its computer</h3>
            <p><strong>€17.99 a month</strong> + VAT for 2 vCores, 4 GB memory and a 50 GB disk, billed from the balance
              every two weeks (€8.30). Enlarge it any time; it keeps everything.</p>
          </div>
          <div class="card">
            <h3>Its brain</h3>
            <p>It starts on DeepSeek, paid from the balance at the AI provider’s price + 10%. Ask the bia for any other
              brain, or to switch to your own key or ChatGPT subscription.</p>
          </div>
          <div class="card">
            <h3>To start</h3>
            <p>The first two weeks of its computer and €10 on the balance: <strong>€18.30</strong> + VAT. Unused money
              carries over; the first bia’s payment comes back within 14 days if you change your mind.</p>
          </div>
        </div>
        <p class="cloud__fine">Prices are drafts during early access. The current ones are always on <a :href="`${CLOUD}/pricing/`">the bia cloud pricing page</a>.</p>
      </div>
    </section>

    <section class="section">
      <div class="container">
        <span class="eyebrow">Still yours</span>
        <h2 class="section-title">We run the computer. The bia is yours.</h2>
        <ul class="list-check cloud__list">
          <li><strong>Only you hold its key.</strong> You collect it once when it wakes up; we delete our copy. Nobody else can open the bia — us included.</li>
          <li><strong>We don’t look inside.</strong> Its memories, chats, passwords and accounts stay on its own computer.</li>
          <li><strong>Take it home any time.</strong> Ask it to fold itself and download the file — then wake it up on a USB stick or a server of your own with <NuxtLink to="/download">bia deploy</NuxtLink>.</li>
          <li><strong>Backed up every night</strong>, and a person answers in the support desk when something looks wrong with its server.</li>
          <li><strong>Several bias, one account.</strong> Share the balance between their brains, and see what each one spent.</li>
        </ul>
      </div>
    </section>

    <section class="section section--sky">
      <div class="narrow cloud__or">
        <h2 class="section-title">Or run it yourself, free</h2>
        <p class="section-lead">
          bia is free software. A USB stick and a computer you are not using are enough —
          <NuxtLink to="/download">get bia</NuxtLink>, or put it on <NuxtLink to="/servers">a server you rent</NuxtLink>.
        </p>
      </div>
    </section>

    <CtaBand title="Ready when you are" text="Wake one up in the cloud in about half an hour, or on your own computer for free." />
  </main>
</template>

<script setup lang="ts">
const CLOUD = 'https://cloud.ownbia.com'

useSeoMeta({
  title: 'bia cloud — a bia on its own server, run for you',
  description:
    'bia cloud gives a bia its own server, kept running and backed up, with pay-as-you-go billing from €17.99 a month. Only you hold its key; take it home any time.',
})

const steps = [
  { title: 'Name it', text: 'Choose its name and the language it should greet you in. It learns about a hundred more later.' },
  { title: 'Pay once', text: 'The first two weeks of its computer and a little balance for its brain. Your account is made with the payment.' },
  { title: 'Say hello', text: 'Its own server starts and bia moves in — about half an hour. Collect its key, open it in the bia deploy app, and you are off.' },
]
</script>

<style scoped>
.steps .card { position: relative; }
.step__n { display: inline-grid; place-items: center; width: 2rem; height: 2rem; border-radius: 50%; background: var(--accent); color: #fff; font-weight: 800; margin-bottom: 0.6rem; }
.cloud__fine { margin-top: 1.2rem; color: var(--mute); font-size: 0.9rem; }
.cloud__list { max-width: 48rem; }
.cloud__or { text-align: center; }
</style>
