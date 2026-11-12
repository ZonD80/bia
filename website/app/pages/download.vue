<template>
  <main>
    <section class="page-hero">
      <div class="container">
        <nav class="crumbs" aria-label="Breadcrumb">
          <NuxtLink to="/">Home</NuxtLink><span aria-hidden="true">/</span><span>Get bia</span>
        </nav>
        <span class="eyebrow">Get bia</span>
        <h1>Give a bia a place to live</h1>
        <p class="section-lead">
          A bia is not an app you install on your computer. It is a whole computer of its own. So getting one
          means choosing where it lives: on a server we run for you, on a server you rent, or on a computer you
          already have. Wherever it starts, it can move later as the same someone.
        </p>
        <nav class="jump" aria-label="Jump to">
          <a class="chip" href="#cloud">bia cloud</a>
          <a class="chip" href="#installers">Installers</a>
          <a class="chip" href="#life">How a bia lives</a>
          <a class="chip" href="#stick">USB stick, step by step</a>
        </nav>
      </div>
    </section>

    <!-- 1. cloud -->
    <section id="cloud" class="section anchor">
      <div class="container">
        <div class="cloud card">
          <div>
            <span class="eyebrow">The easy way</span>
            <h2>bia cloud: we run its computer</h2>
            <p>
              Give it a name and a language, pay once, and about half an hour later your bia is awake on its own
              server, with a brain included. We keep the server running and backed up. Only you hold its key, we
              do not look inside, and you can take it home to your own computer any time.
            </p>
            <ul class="list-check">
              <li>From €17.99 a month for its computer, billed every two weeks from a balance</li>
              <li>Thinking is paid as it is used; switch to your own AI account by asking bia</li>
              <li>Nothing to install: you talk to it from your phone or browser</li>
            </ul>
            <div class="btn-row">
              <a class="btn btn--primary" href="https://cloud.ownbia.com" target="_blank" rel="noopener">Start a bia in the cloud</a>
              <NuxtLink class="btn btn--ghost" to="/cloud">About bia cloud</NuxtLink>
            </div>
          </div>
          <div class="cloud__art" aria-hidden="true"><PotatoMark label="" /></div>
        </div>
      </div>
    </section>

    <!-- 2. installers -->
    <section id="installers" class="section section--tint anchor">
      <div class="container">
        <span class="eyebrow">Your own hardware</span>
        <h2 class="section-title">Installers: bia deploy</h2>
        <p class="section-lead">
          Free and open source. bia deploy puts a bia on a server you rent, with no commands to type, or writes it
          onto a USB stick that turns any PC or laptop into one.
        </p>
        <h3 class="h3">On a computer</h3>
        <p class="h3note">Servers and USB sticks.</p>
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
        <h3 class="h3">On a phone</h3>
        <p class="h3note">Servers only. The same app is then your chat with bia.</p>
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
          <div class="card host">
            <h3>iPhone</h3>
            <p class="host__note">iPhone and iPad</p>
            <a class="btn btn--primary host__btn" :href="APP_STORE_URL" target="_blank" rel="noopener">Download on the App Store</a>
            <a class="btn btn--ghost host__btn host__btn--alt" :href="TESTFLIGHT_URL" target="_blank" rel="noopener">Join the TestFlight beta</a>
            <a class="btn btn--ghost host__btn host__btn--alt" :href="APPDB_URL" target="_blank" rel="noopener">Get it on appdb</a>
            <p class="host__ver">App Store · TestFlight for betas · appdb is another install path</p>
          </div>
          <div class="card host">
            <h3>Other ways in</h3>
            <p class="host__note">For the technical</p>
            <ul class="ways">
              <li><strong>One command</strong> on a running Linux server: <NuxtLink to="/servers">the server guide</NuxtLink></li>
              <li><strong>On a Mac</strong>, in a virtual machine: <a href="https://github.com/ZonD80/clawhome/releases" target="_blank" rel="noopener">ClawHome</a></li>
              <li><strong>From source</strong>, GPL-3.0: <a :href="REPO_URL" target="_blank" rel="noopener">GitHub</a></li>
            </ul>
          </div>
        </div>
        <p v-if="version" class="release-note">
          Latest bia: version {{ version }} · <a :href="releaseUrl">release notes</a>
        </p>
        <p v-if="error" class="release-note release-note--warn">
          {{ error }} — <a :href="fallbackUrl">open the releases page</a>
        </p>
      </div>
    </section>

    <!-- 3. how a bia lives -->
    <section id="life" class="section anchor">
      <div class="container">
        <span class="eyebrow">How a bia lives</span>
        <h2 class="section-title">Where it can live, and what it does there by itself</h2>
        <p class="section-lead">
          The computer is bia’s body, not its identity. Its name, memories, people, accounts and skills are one
          folder that it looks after and can carry elsewhere.
        </p>

        <h3 class="h3">Where it can live</h3>
        <div class="grid grid--feat homes">
          <div v-for="h in homes" :key="h.title" class="card home">
            <AppIcon :name="h.icon" size="1.8rem" class="ico" />
            <h4>{{ h.title }}</h4>
            <p>{{ h.text }}</p>
          </div>
        </div>

        <h3 class="h3">What it does there</h3>
        <div class="grid grid--2 lives">
          <div v-for="l in lives" :key="l.title" class="card life">
            <AppIcon :name="l.icon" size="2rem" class="ico" />
            <div>
              <h4>{{ l.title }}</h4>
              <p>{{ l.text }}</p>
              <NuxtLink v-if="l.to" :to="l.to">{{ l.more }}</NuxtLink>
            </div>
          </div>
        </div>
      </div>
    </section>

    <!-- 4. the stick, step by step -->
    <section id="stick" class="section section--tint anchor">
      <div class="container">
        <span class="eyebrow">Step by step</span>
        <h2 class="section-title">A computer you already have</h2>
        <p class="section-lead">
          About twenty minutes: download bia deploy above, choose a brain, write a USB stick, start the computer
          from it. bia wakes up and greets you.
        </p>
        <h3 class="h3">Choose how bia should think</h3>
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

        <h3 class="h3">Write the stick and start a computer</h3>
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

    <CtaBand title="Next: introduce it" text="Tell it its name, give it a phone number of its own, and add it to the family chat." />
  </main>
</template>

<script setup lang="ts">
useSeoMeta({
  title: 'Get bia — in the cloud, on your server or an old computer',
  description: 'Where a bia can live and how to get one: bia cloud runs it for you; bia deploy for Mac, Windows, Linux, Android and iPhone puts it on your server or a USB stick. How a bia moves, updates, repairs and develops itself, and sets up other bias.',
})

import type { FlashHost } from '~/composables/useLatestRelease'

type Host = { id: FlashHost; label: string; note: string }

// the phone apps (server only)
const PLAY_URL = 'https://play.google.com/store/apps/details?id=biadeploy.android'
const APP_STORE_URL = 'https://apps.apple.com/app/bia-deploy/id6827948196'
const TESTFLIGHT_URL = 'https://testflight.apple.com/join/qePtQWqF'
const APPDB_URL = 'https://appdb.to/details/c25d9b6ff3034e039c294b1acd92eafcfc14c9b0'

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

const homes = [
  { icon: 'globe', title: 'bia cloud', text: 'A server we keep running and backed up. Awake in about half an hour.' },
  { icon: 'link', title: 'A server you rent', text: 'Any Linux server or VPS. bia deploy replaces its system with bia.' },
  { icon: 'terminal', title: 'An old PC or laptop', text: 'Almost any Intel or AMD machine of the last ten or more years, or an ARM mini-PC.' },
  { icon: 'stick', title: 'A USB stick', text: 'bia can stay on the stick and wake up on whatever computer it is plugged into.' },
  { icon: 'window', title: 'A virtual machine', text: 'On a Mac with ClawHome, or in any hypervisor from the install image.' },
]

const lives = [
  { icon: 'move', title: 'It moves', text: 'bia folds itself into one archive — memories, chats, accounts, passwords, browser, skills — and unfolds on another machine as the same someone: from a stick onto a disk, from a laptop to a server, from Intel to ARM, from bia cloud to your home. It can fly across the network by itself.', to: '/features/moving', more: 'How moving works' },
  { icon: 'refresh', title: 'It updates and repairs itself', text: 'It installs its own system updates, then checks what the update broke and fixes it before telling you. Its files are checked at every start, its settings keep a last good copy, and it makes its own backups.', to: '/features/updates', more: 'How updates work' },
  { icon: 'clock', title: 'It acts on its own', text: 'bia has a clock and a calendar. It keeps schedules, watches inboxes and pages, writes to people first when something is due, and stays quiet when there is nothing to say. After being switched off it catches up on everything it missed.', to: '/features/calendar', more: 'Time and reminders' },
  { icon: 'puzzle', title: 'It develops itself', text: 'When a tool is missing, bia writes it: a new skill, a connection to a service, a watcher for something you care about. Each one lives in its own folder with a description, survives updates and can be handed to another bia.', to: '/features/develops-itself', more: 'How it grows' },
  { icon: 'people', title: 'It sets up new bias', text: 'Give a bia the login of a rented server and it will wake a new bia there by itself — for the shop, for the grandparents — or prepare a place and fly there. Bias recognise each other by certificate and talk on their own link.', to: '/servers', more: 'Bias and servers' },
  { icon: 'brain', title: 'It chooses how to think', text: 'Its brain is an AI model you pick and can change by asking: a ChatGPT plan, an inexpensive pay-as-you-go model, or a model running on its own powerful computer without the internet. If the brain fails, it falls back to the previous one and tells you.', to: '/features/brain', more: 'About the brain' },
  { icon: 'key', title: 'It keeps its own keys', text: 'Passwords, one-time codes and passkeys for bia’s own accounts are in a safe on its machine. You reach bia with a personal certificate; nobody else gets in, including us.', to: '/features/vault', more: 'The safe' },
  { icon: 'shield', title: 'It stays separate', text: 'bia never asks for the keys to your computer or your accounts. It has its own number, email and logins, and you share with it what you would share with a colleague.', to: '/features/privacy', more: 'Privacy' },
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
