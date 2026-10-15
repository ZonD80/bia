<template>
  <main>
    <!-- hero -->
    <section class="hero">
      <div class="container hero__grid">
        <div class="hero__text rise">
          <span class="eyebrow">Boot into agent</span>
          <h1>Turn anything into <em>someone</em>.</h1>
          <p class="hero__lead">
            bia is a separate mind with a computer of its own. It answers your family on WhatsApp,
            understands about a hundred languages on calls, remembers what matters, uses the web and its own
            computer — and when it is time, it moves to a new machine as the same personality.
          </p>
          <div class="btn-row">
            <NuxtLink class="btn btn--primary" to="/download">Get bia — it’s free</NuxtLink>
            <NuxtLink class="btn btn--ghost" to="/features">See everything it does</NuxtLink>
          </div>
          <p class="hero__small">Runs on a USB stick · old laptops welcome · your data stays with you</p>
        </div>
        <div class="hero__art" aria-hidden="true">
          <div class="hero__glow" />
          <div class="hero__stage"><div class="hero__potato"><PotatoMark label="" /></div></div>
          <div class="hero__bubble hero__bubble--a">“Mom asked about Sunday lunch — I said 1 pm, okay?”</div>
          <div class="hero__bubble hero__bubble--b">🎤 Hola, ¿a qué hora abren?</div>
          <div class="hero__bubble hero__bubble--c">Parcel arrives tomorrow 10–12 📦</div>
        </div>
      </div>
    </section>

    <!-- what is it -->
    <section class="section">
      <div class="container">
        <span class="eyebrow">What is bia?</span>
        <h2 class="section-title">Not a program on your PC. A separate someone.</h2>
        <p class="section-lead">
          Most AI assistants move into your computer. They read your files, act with your accounts, work inside
          limits someone else set — and whatever they can reach, they can one day leak. bia is the opposite: a
          separate mind, like a person you work with. It has its own computer, its own files, its own accounts and
          contacts, its own social life. You share what you choose, the way you would with a colleague — and your
          own PC stays yours.
        </p>
        <div class="grid grid--3 what">
          <div class="card">
            <AppIcon name="chat" size="2rem" class="what__ico" />
            <h3>It meets people where they are</h3>
            <p>On WhatsApp, Telegram, phone calls and voice messages — in their own language. Nobody has to install anything.</p>
          </div>
          <div class="card">
            <AppIcon name="terminal" size="2rem" class="what__ico" />
            <h3>It really does things</h3>
            <p>It has a whole computer to work with: programs, files, a web browser, the screen, a safe for passwords.</p>
          </div>
          <div class="card">
            <AppIcon name="move" size="2rem" class="what__ico" />
            <h3>It has a life of its own</h3>
            <p>Its own machine, files, memories and connections — never the keys to yours. It moves to new computers and updates itself without forgetting a thing.</p>
          </div>
        </div>
      </div>
    </section>

    <!-- a day with bia -->
    <section class="section section--tint">
      <div class="container day">
        <div>
          <span class="eyebrow">A day with bia</span>
          <h2 class="section-title">What an ordinary Tuesday looks like</h2>
          <p class="section-lead">
            Meet Lena. She runs a small guesthouse and lives with her son and her mother. An old laptop in the
            cupboard runs bia, thinking with an inexpensive cloud brain.
          </p>
        </div>
        <ol class="day__list">
          <li v-for="(s, i) in day" :key="i">
            <span class="day__time">{{ s.time }}</span>
            <div>
              <p class="day__what">{{ s.what }}</p>
              <NuxtLink class="chip" :to="`/features/${s.feature}`">{{ featureName(s.feature) }}</NuxtLink>
            </div>
          </li>
        </ol>
      </div>
    </section>

    <!-- all features -->
    <section class="section">
      <div class="container">
        <span class="eyebrow">Everything it does</span>
        <h2 class="section-title">Twenty abilities, one personality</h2>
        <p class="section-lead">Each one has its own page with examples. Here is the whole picture.</p>
        <div v-for="(g, key) in groups" :key="key" class="fgroup">
          <div class="fgroup__head">
            <h3>{{ g.title }}</h3>
            <p>{{ g.blurb }}</p>
          </div>
          <div class="grid grid--feat">
            <FeatureCard v-for="f in byGroup(key)" :key="f.slug" :feature="f" />
          </div>
        </div>
      </div>
    </section>

    <!-- conversation showcase -->
    <section class="section section--sky">
      <div class="container show">
        <div>
          <span class="eyebrow">Real human interaction</span>
          <h2 class="section-title">It talks like a thoughtful person</h2>
          <p class="section-lead">
            bia keeps track of every conversation. When something takes time, it says so instead of going silent.
            When it learns a new language, it tells you. When a decision is yours, it asks.
          </p>
          <ul class="list-check">
            <li>Answers each person in the chat they wrote from</li>
            <li>Sends a short status when a job takes longer</li>
            <li>Remembers who people are and how they like to talk</li>
            <li>Catches up on everything after being switched off</li>
          </ul>
        </div>
        <ChatDemo :lines="showcase" title="On Telegram" />
      </div>
    </section>

    <!-- use cases -->
    <section class="section">
      <div class="container">
        <span class="eyebrow">Ideas</span>
        <h2 class="section-title">Who is bia for?</h2>
        <p class="section-lead">Anyone with an old computer and people to help. A few places to start:</p>
        <div class="grid grid--2 uc">
          <NuxtLink v-for="g in useCaseGroups" :key="g.slug" class="uc__card" :to="`/use-cases#${g.slug}`">
            <span class="uc__ico"><AppIcon :name="g.icon" size="1.8rem" /></span>
            <div>
              <h3>{{ g.title }}</h3>
              <p>{{ g.intro }}</p>
            </div>
          </NuxtLink>
        </div>
        <div class="btn-row center">
          <NuxtLink class="btn btn--ghost" to="/use-cases">All {{ totalCases }} use cases</NuxtLink>
        </div>
      </div>
    </section>

    <!-- shape shifting -->
    <section class="section section--tint">
      <div class="container shift">
        <div class="shift__art" aria-hidden="true">
          <div class="shift__box"><AppIcon name="stick" size="2.4rem" /><span>USB stick</span></div>
          <AppIcon name="arrow" size="1.6rem" class="shift__arrow" />
          <div class="shift__box"><AppIcon name="terminal" size="2.4rem" /><span>Old laptop</span></div>
          <AppIcon name="arrow" size="1.6rem" class="shift__arrow" />
          <div class="shift__box"><AppIcon name="window" size="2.4rem" /><span>New mini-PC</span></div>
        </div>
        <div>
          <span class="eyebrow">Shape-shifting</span>
          <h2 class="section-title">Same mind, any body</h2>
          <p class="section-lead">
            bia can pack itself up — memories, chats, passwords, browser, skills — and wake up on another
            computer as exactly the same someone. From a stick onto a laptop’s disk, across the network to a new
            machine, from an Intel PC to an ARM one. It updates itself the same careful way.
          </p>
          <div class="btn-row">
            <NuxtLink class="btn btn--ghost" to="/features/moving">How it moves</NuxtLink>
            <NuxtLink class="btn btn--ghost" to="/features/updates">How it updates</NuxtLink>
          </div>
        </div>
      </div>
    </section>

    <!-- how to start -->
    <section class="section">
      <div class="container">
        <span class="eyebrow">Getting started</span>
        <h2 class="section-title">Three steps, about twenty minutes</h2>
        <div class="grid grid--3 steps">
          <div v-for="(s, i) in steps" :key="i" class="card step">
            <span class="step__n">{{ i + 1 }}</span>
            <AppIcon :name="s.icon" size="2rem" class="what__ico" />
            <h3>{{ s.title }}</h3>
            <p>{{ s.text }}</p>
          </div>
        </div>
      </div>
    </section>

    <CtaBand />
  </main>
</template>

<script setup lang="ts">
import { features, groups, type Feature } from '~/data/features'
import { useCaseGroups } from '~/data/usecases'

useSeoMeta({
  title: 'bia — turn anything into someone',
  description:
    'bia is a separate mind with its own computer, files and accounts — not an assistant inside your PC. It lives on WhatsApp and Telegram, answers calls in 100 languages, remembers, keeps time, uses the web — and moves between computers as the same personality.',
})

const byGroup = (g: string) => features.filter((f) => f.group === (g as Feature['group']))
const featureName = (slug: string) => features.find((f) => f.slug === slug)?.name ?? slug
const totalCases = useCaseGroups.reduce((n, g) => n + g.cases.length, 0)

const day = [
  { time: '07:00', what: 'bia checks the weather and tells the family group to take umbrellas.', feature: 'calendar' },
  { time: '07:40', what: 'A guest from Brazil sends a voice message in Portuguese. bia has never heard Portuguese — it learns it on the spot and answers in her language.', feature: 'languages' },
  { time: '09:15', what: 'Last week Lena asked bia to look after her mother’s pills. Right on time, bia calls Grandma on WhatsApp: “Good morning! Did you take your pills?” — and lets Lena know she did.', feature: 'calendar' },
  { time: '11:30', what: 'bia signs in to the booking website with its saved password and confirms tomorrow’s arrivals.', feature: 'browser' },
  { time: '14:00', what: 'A guest asks for a late checkout. That’s Lena’s call — bia sends her two buttons: “Yes, until 2 pm” · “Sorry, not possible”.', feature: 'ask-human' },
  { time: '17:20', what: 'The internet goes down for an hour. When the line is back, bia catches up on every message that arrived meanwhile and answers each guest in turn — nobody is left waiting.', feature: 'presence' },
  { time: '21:00', what: 'bia quietly writes in its diary what happened today and what is still open for tomorrow.', feature: 'memory' },
]

const showcase = [
  { from: 'them' as const, where: 'Telegram', text: 'Can you find a plumber who works Saturdays?' },
  { from: 'bia' as const, where: 'Telegram', text: 'Looking now — I’ll come back in a few minutes.' },
  { from: 'note' as const, text: 'bia searches, compares reviews, checks opening hours.' },
  { from: 'bia' as const, where: 'Telegram', text: 'Still on it: two candidates so far, checking a third.' },
  { from: 'bia' as const, where: 'Telegram', text: 'Best option: Kovač & Sons, Saturdays 9–14, 4.8★. Want me to write to them for you?' },
  { from: 'them' as const, where: 'Telegram', text: 'Yes please 🙏' },
]

const steps = [
  { icon: 'usb', title: 'Write a USB stick', text: 'Download the small bia app for Mac, Windows or Linux and write bia onto any USB stick of 8 GB or more.' },
  { icon: 'power', title: 'Start a computer from it', text: 'Plug the stick into a PC or laptop and switch it on. Instead of the usual desktop, bia wakes up.' },
  { icon: 'wave', title: 'Say hello', text: 'Type on the screen, or open bia’s private app on your phone. Ask it to join WhatsApp or Telegram, and you’re off.' },
]
</script>

<style scoped>
.hero {
  position: relative;
  overflow: hidden;
  background: linear-gradient(180deg, var(--sky) 0%, var(--paper) 100%);
  padding-block: clamp(3rem, 8vw, 6rem) clamp(3rem, 7vw, 5rem);
}
.hero__grid {
  display: grid;
  gap: 2.5rem;
  align-items: center;
  grid-template-columns: 1.15fr 1fr;
}
.hero h1 {
  font-size: clamp(2.6rem, 6.5vw, 4.6rem);
  line-height: 1.02;
  max-width: 13ch;
}
.hero h1 em {
  font-style: italic;
  color: var(--accent);
}
.hero__lead {
  font-size: clamp(1.08rem, 2vw, 1.25rem);
  color: var(--ink-soft);
  max-width: 36rem;
  margin-bottom: 1.75rem;
}
.hero__small {
  margin-top: 1.1rem;
  font-size: 0.9rem;
  color: var(--mute);
}
.hero__art {
  position: relative;
  min-height: 22rem;
}
.hero__glow {
  position: absolute;
  inset: 10% 5%;
  background: radial-gradient(circle, rgba(199, 154, 92, 0.35), transparent 65%);
  filter: blur(20px);
}
.hero__stage {
  position: absolute;
  inset: 0;
  display: flex;
  align-items: center;
  justify-content: center;
}
.hero__potato {
  width: min(24rem, 85%);
  animation: float 8s ease-in-out infinite;
}
.hero__bubble {
  position: absolute;
  max-width: 15rem;
  padding: 0.65rem 0.9rem;
  border-radius: 1rem;
  background: #fff;
  border: 1px solid var(--line);
  box-shadow: var(--shadow);
  font-size: 0.9rem;
  line-height: 1.4;
  animation: rise 0.9s ease-out both;
}
.hero__bubble--a { top: 2%; left: 0; animation-delay: 0.4s; border-bottom-left-radius: 0.3rem; }
.hero__bubble--b { top: 38%; right: -2%; animation-delay: 0.8s; background: #dfeaf6; border-bottom-right-radius: 0.3rem; }
.hero__bubble--c { bottom: 2%; left: 8%; animation-delay: 1.2s; border-bottom-left-radius: 0.3rem; }

.what { margin-top: 2rem; }
.what__ico { color: var(--accent); margin-bottom: 0.8rem; }

.day {
  display: grid;
  gap: 2.5rem;
  grid-template-columns: 1fr 1.3fr;
  align-items: start;
}
.day > div:first-child {
  position: sticky;
  top: 6rem;
}
.day__list {
  list-style: none;
  margin: 0;
  padding: 0;
  display: grid;
  gap: 1.1rem;
  border-left: 2px solid rgba(63, 111, 42, 0.25);
  padding-left: 1.4rem;
}
.day__list li {
  display: grid;
  grid-template-columns: 3.6rem 1fr;
  gap: 0.75rem;
  position: relative;
}
.day__list li::before {
  content: '';
  position: absolute;
  left: -1.85rem;
  top: 0.45rem;
  width: 0.8rem;
  height: 0.8rem;
  border-radius: 50%;
  background: var(--accent);
  box-shadow: 0 0 0 4px var(--paper-2);
}
.day__time {
  font-family: var(--font-display);
  font-weight: 600;
  color: var(--accent-deep);
}
.day__what {
  margin: 0 0 0.35rem;
}

.fgroup { margin-top: 2.75rem; }
.fgroup__head { display: flex; flex-wrap: wrap; align-items: baseline; gap: 0.25rem 1rem; margin-bottom: 1rem; }
.fgroup__head h3 { font-size: 1.5rem; margin: 0; }
.fgroup__head p { margin: 0; color: var(--mute); }

.show {
  display: grid;
  gap: 2.5rem;
  grid-template-columns: 1fr 1fr;
  align-items: center;
}
.show .list-check { margin-top: 1.25rem; }

.uc { margin-top: 2rem; grid-template-columns: repeat(auto-fit, minmax(min(100%, 15.5rem), 1fr)); }
.uc__card { flex-direction: column; }
.uc__card {
  display: flex;
  gap: 1rem;
  padding: 1.3rem;
  background: var(--card);
  border: 1px solid var(--line);
  border-radius: var(--radius);
  color: inherit;
  text-decoration: none;
  transition: border-color 0.2s ease, transform 0.2s ease;
}
.uc__card:hover { border-color: rgba(63, 111, 42, 0.35); transform: translateY(-2px); }
.uc__card h3 { font-size: 1.15rem; margin-bottom: 0.25rem; }
.uc__card p { margin: 0; color: var(--ink-soft); font-size: 0.96rem; }
.uc__ico {
  display: inline-flex;
  align-items: center;
  justify-content: center;
  width: 3rem;
  height: 3rem;
  border-radius: 0.9rem;
  background: var(--tan-soft);
  color: #8a5f2c;
  flex-shrink: 0;
}
.center { justify-content: center; margin-top: 2rem; }

.shift {
  display: grid;
  gap: 2.5rem;
  grid-template-columns: 1fr 1.2fr;
  align-items: center;
}
.shift__art {
  display: flex;
  align-items: center;
  justify-content: center;
  flex-wrap: wrap;
  gap: 0.75rem;
}
.shift__box {
  display: flex;
  flex-direction: column;
  align-items: center;
  gap: 0.4rem;
  padding: 1rem;
  min-width: 6.5rem;
  background: var(--card);
  border: 1px solid var(--line);
  border-radius: var(--radius);
  color: var(--accent-deep);
  font-size: 0.85rem;
  font-weight: 700;
}
.shift__arrow { color: var(--tan); }

.steps { margin-top: 2rem; }
.step { position: relative; }
.step__n {
  position: absolute;
  top: 1rem;
  right: 1.2rem;
  font-family: var(--font-display);
  font-size: 2.6rem;
  font-weight: 700;
  color: var(--accent-soft);
  line-height: 1;
}

@media (max-width: 900px) {
  .hero__grid, .day, .show, .shift { grid-template-columns: 1fr; }
  /* one column: the intro would stick over the timeline while it scrolls */
  .day > div:first-child { position: static; }
  .hero__art { min-height: 18rem; order: -1; }
  .hero__bubble { font-size: 0.82rem; max-width: 12rem; }
  /* keep the potato's eye visible on narrow screens */
  .hero__bubble--b { top: auto; bottom: 22%; right: 0; }
}
</style>
