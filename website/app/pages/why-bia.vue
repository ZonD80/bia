<template>
  <main>
    <section class="page-hero">
      <div class="container">
        <nav class="crumbs" aria-label="Breadcrumb">
          <NuxtLink to="/">Home</NuxtLink><span aria-hidden="true">/</span><span>Why bia</span>
        </nav>
        <span class="eyebrow">Not another chat box</span>
        <h1>Why bia is different from the AI you know</h1>
        <p class="section-lead">
          Today’s AI assistants are clever, but they live in someone else’s website and vanish when you close the
          tab. bia is built the other way around: a mind with its own computer, its own social life, and the
          judgement to know when to ask.
        </p>
      </div>
    </section>

    <section class="section">
      <div class="container">
        <div class="compare" role="table" aria-label="Typical AI assistant compared with bia">
          <div class="compare__row compare__row--head" role="row">
            <span role="columnheader" />
            <span role="columnheader">Typical AI assistant</span>
            <span role="columnheader">bia</span>
          </div>
          <div v-for="r in compare" :key="r.topic" class="compare__row" role="row">
            <span class="compare__topic" role="rowheader">{{ r.topic }}</span>
            <span role="cell">{{ r.them }}</span>
            <span role="cell" class="compare__bia">{{ r.bia }}</span>
          </div>
        </div>
      </div>
    </section>

    <section class="section section--tint">
      <div class="container">
        <h2 class="section-title">A week in bia’s life</h2>
        <p class="section-lead">From bia’s own notes during a seven-day test run on a real computer.</p>
        <ol class="week">
          <li v-for="(d, i) in week" :key="i" class="card">
            <span class="week__day">{{ d.day }}</span>
            <h3>{{ d.title }}</h3>
            <p>{{ d.text }}</p>
          </li>
        </ol>
      </div>
    </section>

    <section class="section">
      <div class="container">
        <h2 class="section-title">Character, not just capability</h2>
        <div class="grid grid--3">
          <div v-for="c in character" :key="c.title" class="card">
            <AppIcon :name="c.icon" size="2rem" class="ico" />
            <h3>{{ c.title }}</h3>
            <p>{{ c.text }}</p>
          </div>
        </div>
      </div>
    </section>

    <section class="section section--tint">
      <div class="narrow">
        <h2>Where bia comes from</h2>
        <p>
          bia grew out of two earlier projects. One taught an AI to build and drive a whole second computer inside
          itself; the other taught it to live in messengers as a person, with contacts, trust levels and voice
          calls. bia combines both into a single being that owns its computer — and adds memory, moving, updating
          and a hundred languages.
        </p>
      </div>
    </section>

    <CtaBand />
  </main>
</template>

<script setup lang="ts">
useSeoMeta({
  title: 'Why bia is different from typical AI assistants',
  description:
    'How bia differs from chat-box AI: its own computer, a real social presence on WhatsApp and Telegram, memory that lasts, background work, moving between computers and self-repair.',
})

const compare = [
  { topic: 'Where it lives', them: 'In a company’s website or app', bia: 'On a computer you own — even an old one' },
  { topic: 'How people reach it', them: 'You go to its app', bia: 'It comes to WhatsApp, Telegram, calls — where people already are' },
  { topic: 'Memory', them: 'Forgets when the chat ends', bia: 'Keeps a diary and reflections across days, restarts and moves' },
  { topic: 'Doing things', them: 'Talks about what it would do', bia: 'Uses a real computer, browser and screen to actually do it' },
  { topic: 'Long tasks', them: 'Freezes until a step finishes', bia: 'Works in the background and keeps talking' },
  { topic: 'Being away', them: 'Messages are simply missed', bia: 'Catches up on everything that arrived while it was off' },
  { topic: 'Languages', them: 'Text in many languages', bia: 'Hears and speaks them on calls, and learns new ones itself' },
  { topic: 'Your data', them: 'On their servers', bia: 'On bia’s own disk; only the thinking goes to the cloud brain you pick' },
  { topic: 'Hardware changes', them: 'Not its problem', bia: 'Moves to new hardware and repairs itself' },
]

const week = [
  { day: 'Day 0', title: 'Moves off the stick', text: 'One sentence — “move to the disk” — and bia found the drive, copied itself and woke up from the internal disk.' },
  { day: 'Day 1', title: 'Builds its senses', text: 'Installed its browser, connected its skills, and signed in to Google like a person: codes from its vault, a captcha from a human.' },
  { day: 'Day 2', title: 'Watches without nagging', text: 'Wrote a watcher for mail and calendar that only speaks up when something actually matters.' },
  { day: 'Day 3–4', title: 'Gets a face', text: 'Set its profile photos, turned on two-factor login, opened accounts, posted — and fixed a bug in its own tools along the way.' },
  { day: 'Day 5', title: 'Makes a phone call', text: 'Placed and answered voice calls, joined a family group, and learned who is who.' },
  { day: 'Day 6–7', title: 'Holds its ground', text: 'Politely refused a stranger who demanded access to the computer, then repaired itself after being moved onto different hardware.' },
]

const character = [
  { icon: 'hand', title: 'Knows its limits', text: 'Captchas and decisions that are yours go to you. Risky steps wait for your yes. Things it cannot do, it says it cannot do.' },
  { icon: 'shield', title: 'Holds boundaries', text: 'Strangers cannot talk it into giving away your computer or your secrets. Pressure is reported, not obeyed.' },
  { icon: 'people', title: 'Treats people as people', text: 'It remembers corrections, introductions and preferences — and stays the same character with everyone.' },
]
</script>

<style scoped>
.compare {
  display: grid;
  border: 1px solid var(--line);
  border-radius: var(--radius);
  overflow: hidden;
  background: var(--card);
}
.compare__row {
  display: grid;
  grid-template-columns: 1fr 1.4fr 1.6fr;
  gap: 1rem;
  padding: 1rem 1.25rem;
  border-top: 1px solid var(--line);
  align-items: start;
}
.compare__row--head {
  border-top: 0;
  background: var(--paper-2);
  font-weight: 700;
  font-size: 0.9rem;
  color: var(--mute);
}
.compare__topic {
  font-weight: 700;
}
.compare__bia {
  color: var(--accent-deep);
  font-weight: 600;
}
.week {
  list-style: none;
  margin: 2rem 0 0;
  padding: 0;
  display: grid;
  gap: 1.1rem;
  grid-template-columns: repeat(auto-fill, minmax(min(100%, 19rem), 1fr));
}
.week__day {
  display: inline-block;
  margin-bottom: 0.5rem;
  font-size: 0.8rem;
  font-weight: 700;
  letter-spacing: 0.08em;
  text-transform: uppercase;
  color: #8a5f2c;
  background: var(--tan-soft);
  padding: 0.2rem 0.6rem;
  border-radius: 999px;
}
.ico { color: var(--accent); margin-bottom: 0.7rem; }
@media (max-width: 720px) {
  .compare__row { grid-template-columns: 1fr; gap: 0.3rem; }
  .compare__row--head { display: none; }
  .compare__row span[role='cell']::before { font-size: 0.75rem; font-weight: 700; color: var(--mute); display: block; }
  .compare__row span[role='cell']:nth-child(2)::before { content: 'Typical AI'; }
  .compare__row span[role='cell']:nth-child(3)::before { content: 'bia'; }
}
</style>
