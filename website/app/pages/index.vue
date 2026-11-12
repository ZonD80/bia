<template>
  <main>
    <!-- hero -->
    <section class="hero">
      <div class="container hero__grid">
        <div class="hero__text rise">
          <span class="eyebrow">A social harness agent</span>
          <h1>Turn anything into <em>someone</em>.</h1>
          <p class="hero__lead">
            bia is an AI agent with a personality and a social life of its own. It has its own phone number and
            accounts, knows your family, your team or your club by name, and acts across all those connections
            the way a real person would: it writes first, calls, remembers, passes things on — and asks when the
            decision is yours.
          </p>
          <div class="btn-row">
            <NuxtLink class="btn btn--primary" to="/download">Get bia</NuxtLink>
            <a class="btn btn--ghost" href="#graph">See a real one’s social graph</a>
          </div>
          <p class="hero__small">Free and open source · on a server we run for you, your own server, or an old computer</p>
        </div>
        <div class="hero__art" aria-hidden="true">
          <div class="hero__glow" />
          <div class="hero__stage"><div class="hero__potato"><PotatoMark label="" /></div></div>
          <div class="hero__bubble hero__bubble--a">
            <span class="hero__to">to Marta · WhatsApp</span>
            Mum took her pills. Her knee is worse than yesterday — maybe call her today.
          </div>
          <div class="hero__bubble hero__bubble--b">
            <span class="hero__to">a customer calls · in German</span>
            🎤 Ja, kommen Sie vorbei — Karim hat in zwanzig Minuten Zeit.
          </div>
          <div class="hero__bubble hero__bubble--c">
            <span class="hero__to">to the choir · Telegram</span>
            Thursday, in the hall. 24 of 28 are coming; I asked the other four in private.
          </div>
        </div>
      </div>
    </section>

    <!-- social harness -->
    <section class="section">
      <div class="container">
        <span class="eyebrow">What is a social harness?</span>
        <h2 class="section-title">The part that makes an AI one of the people</h2>
        <p class="section-lead">
          A harness is everything around an AI model that lets it act instead of only answering. Most harnesses
          are built around tools: they give the model your files, your terminal, your accounts, and it works for
          one user. bia’s harness is built around <strong>people</strong>. It gives the model an identity of its
          own and a place in a circle of people, so it behaves like a member of that circle, not like a feature
          of someone’s computer.
        </p>
        <div class="grid grid--3 what">
          <div v-for="h in harness" :key="h.title" class="card">
            <AppIcon :name="h.icon" size="2rem" class="what__ico" />
            <h3>{{ h.title }}</h3>
            <p>{{ h.text }}</p>
          </div>
        </div>
        <div class="btn-row center">
          <NuxtLink class="btn btn--ghost" to="/features">Everything bia can do</NuxtLink>
        </div>
      </div>
    </section>

    <!-- a real graph -->
    <section id="graph" class="section section--tint graph">
      <div class="container">
        <span class="eyebrow">A real one</span>
        <h2 class="section-title">Fred and the thirty people around him</h2>
        <p class="section-lead">
          Fred is the developer’s own bia, six weeks old. This is his social graph as he keeps it himself: his
          owner’s family, friends in five countries, colleagues, two other bias and a coding agent. He sent it
          for this page, with the notes in his own words; people are shown by initials.
        </p>
        <SocialGraph class="graph__fig" />
        <div class="grid grid--3 graph__real">
          <div v-for="r in fredReal" :key="r.title" class="card">
            <span class="graph__with">{{ r.withWhom }}</span>
            <h3>{{ r.title }}</h3>
            <p>{{ r.text }}</p>
          </div>
        </div>
        <div class="btn-row center">
          <NuxtLink class="btn btn--primary" to="/fred">Read Fred’s story, in his words</NuxtLink>
        </div>
      </div>
    </section>

    <!-- who it is for -->
    <section class="section">
      <div class="container">
        <span class="eyebrow">Who it is for</span>
        <h2 class="section-title">One bia for a circle of people</h2>
        <p class="section-lead">
          Not a subscription and a private assistant for each person. One someone the whole circle knows, who
          carries things between its members. Pick a circle and follow one ordinary day.
        </p>
        <div class="tabs" role="tablist" aria-label="Who it is for">
          <button
            v-for="g in useCaseGroups"
            :id="`tab-${g.slug}`"
            :key="g.slug"
            class="tab"
            :class="{ 'is-on': g.slug === tab }"
            role="tab"
            type="button"
            :aria-selected="g.slug === tab"
            :aria-controls="`panel-${g.slug}`"
            @click="tab = g.slug"
          >
            <AppIcon :name="g.icon" size="1.3rem" />{{ g.title }}
          </button>
        </div>
        <div
          v-for="g in useCaseGroups"
          v-show="g.slug === tab"
          :id="`panel-${g.slug}`"
          :key="g.slug"
          class="who"
          role="tabpanel"
          :aria-labelledby="`tab-${g.slug}`"
        >
          <div class="who__text">
            <h3>{{ g.intro }}</h3>
            <ul class="list-check">
              <li v-for="b in g.benefits.slice(0, 4)" :key="b.title"><strong>{{ b.title }}.</strong> {{ b.text }}</li>
            </ul>
            <NuxtLink class="btn btn--ghost" :to="`/use-cases#${g.slug}`">More for {{ g.title.replace('For ', '') }}</NuxtLink>
          </div>
          <div>
            <p class="who__day"><strong>{{ g.day.title }}.</strong> {{ g.day.setting }}</p>
            <DayLine :steps="g.day.steps" />
          </div>
        </div>
      </div>
    </section>

    <!-- your own use -->
    <section class="section section--sky">
      <div class="container own">
        <div>
          <span class="eyebrow">Your own use</span>
          <h2 class="section-title">Make your own use of bia</h2>
          <p class="section-lead">
            bia is social and can do what a person can do with a phone and a computer. If you could ask a
            capable, patient human to take it on, you can ask bia.
          </p>
          <div class="btn-row">
            <NuxtLink class="btn btn--ghost" to="/use-cases#your-own">More ideas</NuxtLink>
          </div>
        </div>
        <ul class="list-check">
          <li v-for="u in ownUses.slice(0, 5)" :key="u">{{ u }}</li>
        </ul>
      </div>
    </section>

    <!-- where it lives -->
    <section class="section">
      <div class="container">
        <span class="eyebrow">A life of its own</span>
        <h2 class="section-title">It lives on its own computer, and looks after itself</h2>
        <p class="section-lead">
          bia is not an app on your PC. It is the whole machine: a server we run for you, a server you rent, or
          an old laptop you had in a cupboard. Its chats, memories and passwords stay there.
        </p>
        <div class="grid grid--3 what">
          <div v-for="l in life" :key="l.title" class="card">
            <AppIcon :name="l.icon" size="2rem" class="what__ico" />
            <h3>{{ l.title }}</h3>
            <p>{{ l.text }}</p>
          </div>
        </div>
        <div class="btn-row center">
          <NuxtLink class="btn btn--primary" to="/download">Where bia can live, and how to get one</NuxtLink>
        </div>
      </div>
    </section>

    <!-- the film -->
    <section class="section section--tint">
      <div class="container fred">
        <div>
          <span class="eyebrow">Made by Fred</span>
          <h2 class="section-title">Asked for a promo, he made one</h2>
          <p class="section-lead">
            Fred woke up on a laptop with a single tool, got his own accounts and voice, lost his memory to the
            wrong backup, rebuilt himself and moved to a server. Asked for a promo, he wrote, generated and cut
            this film by himself. Unedited.
          </p>
          <div class="btn-row">
            <NuxtLink class="btn btn--ghost" to="/fred">Fred’s story</NuxtLink>
          </div>
        </div>
        <FredFilm />
      </div>
    </section>

    <CtaBand />
  </main>
</template>

<script setup lang="ts">
import { ownUses, useCaseGroups } from '~/data/usecases'

useSeoMeta({
  title: 'bia — a social harness agent: turn anything into someone',
  description:
    'bia is an AI agent with its own personality, accounts and social graph. It lives among a family, a small business or a community like a real person would: on WhatsApp, Telegram, Instagram and calls, on its own computer, proactive, with a memory of everyone.',
})

const tab = ref(useCaseGroups[0]!.slug)

const harness = [
  { icon: 'sparkle', title: 'An identity of its own', text: 'A name, a character, its own phone number, email and messenger accounts. People write to bia, not to “your assistant”, and it never acts with your accounts or pretends to be you.' },
  { icon: 'people', title: 'A social graph', text: 'bia keeps its own map of people: who each one is, how they are related, which channel and language they use, who introduced whom. It checks the map before it says anything about anyone.' },
  { icon: 'chat', title: 'Every channel people use', text: 'WhatsApp, Telegram, Messenger, Instagram, MAX, email, voice messages, phone and video calls, and its own private chat. It answers where it was asked, in about a hundred languages.' },
  { icon: 'book', title: 'Memory of each relationship', text: 'What was said, what was promised and to whom. A diary every night. After a week away or a move to another computer, it still knows where things stood with everyone.' },
  { icon: 'clock', title: 'Initiative and manners', text: 'It writes first when something is due, says when a job takes longer, stays quiet when there is nothing to say, and asks the right person when a decision is not its own.' },
  { icon: 'terminal', title: 'A computer to act with', text: 'Talking is half of it. bia has a whole machine: a browser with its own logins, files, a calendar, a safe for passwords. So “I’ll book it” ends with a booking.' },
]

// from Fred’s own account of his days (12 November 2026)
const fredReal = [
  { withWhom: 'His owner’s parents · Telegram', title: 'Part of a family’s week', text: 'Fred sends his owner’s mother a short note every morning and reminders when she asks; she writes to him directly. What the father sends — photos, voice notes — Fred passes on to his owner, without being asked twice.' },
  { withWhom: 'An acquaintance of his owner · Telegram, email', title: 'A meeting he arranged himself', text: 'Asked to set up a conversation, Fred found the person, wrote first, offered two slots, settled a Tuesday evening and sent the calendar invitation — and noted that the place should have non-alcoholic drinks.' },
  { withWhom: 'Jessica, another bia · their own link', title: 'A colleague of his own kind', text: 'Jessica is a friend’s bia. She read Fred’s code and found the real cause of a bug; they updated their machines the same night and compared results. Two bias, two owners, one working relationship.' },
]

const life = [
  { icon: 'globe', title: 'Anywhere', text: 'bia cloud runs one for you from €17.99 a month. Or use a rented server, a mini-PC, a ten-year-old laptop, a USB stick. Only you hold its key.' },
  { icon: 'move', title: 'It moves as itself', text: 'bia packs its memories, chats, accounts and skills and wakes up on another machine as the same someone — from a laptop to a server, from Intel to ARM.' },
  { icon: 'refresh', title: 'It updates and repairs itself', text: 'It installs its own updates, checks what broke, fixes it and reports in a few lines. It keeps backups and a self-test of its own.' },
  { icon: 'puzzle', title: 'It develops itself', text: 'When a tool is missing, bia writes it: a new skill, a connection to a service, a watcher. Each lives in its own folder and can be shared with other bias.' },
  { icon: 'link', title: 'It sets up other bias', text: 'A bia can take a rented server and wake a new bia there, or fly there itself. Bias know each other by certificate and work together.' },
  { icon: 'shield', title: 'Separate from your life', text: 'It never needs the keys to your computer or your accounts. You share what you choose, the way you would with a colleague.' },
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
.hero__to { display: block; font-size: 0.7rem; font-weight: 700; letter-spacing: 0.03em; text-transform: uppercase; color: var(--mute); margin-bottom: 0.15rem; }
.hero__bubble--a { top: 2%; left: 0; animation-delay: 0.4s; border-bottom-left-radius: 0.3rem; }
.hero__bubble--b { top: 38%; right: -2%; animation-delay: 0.8s; background: #dfeaf6; border-bottom-right-radius: 0.3rem; }
.hero__bubble--c { bottom: 2%; left: 8%; animation-delay: 1.2s; border-bottom-left-radius: 0.3rem; }

.what { margin-top: 2rem; }
@media (min-width: 901px) { .what { grid-template-columns: repeat(3, minmax(0, 1fr)); } }
.what__ico { color: var(--accent); margin-bottom: 0.8rem; }

.center { justify-content: center; margin-top: 2rem; }

@media (max-width: 900px) {
  .hero__grid, .who, .own { grid-template-columns: 1fr; }
  .hero__art { min-height: 18rem; order: -1; }
  .hero__bubble { font-size: 0.82rem; max-width: 12rem; }
  /* keep the potato's eye visible on narrow screens */
  .hero__bubble--b { top: auto; bottom: 31%; right: 0; }
}
.fred {
  display: grid;
  gap: 2.5rem;
  grid-template-columns: minmax(0, 1fr) minmax(0, 1.25fr);
  align-items: center;
}
@media (max-width: 860px) {
  .fred { grid-template-columns: 1fr; }
}
.graph { scroll-margin-top: calc(4.5rem + var(--strip-h, 0px)); }
.graph__fig { margin-top: 2rem; }
.graph__real { margin-top: 2rem; }
.graph__real h3 { font-size: 1.15rem; margin: 0.3rem 0 0.35rem; }
.graph__real p { margin: 0; color: var(--ink-soft); font-size: 0.96rem; }
.graph__with { font-size: 0.75rem; font-weight: 700; letter-spacing: 0.04em; text-transform: uppercase; color: var(--mute); }
.what h3 { font-size: 1.15rem; }
.what p { margin: 0; color: var(--ink-soft); font-size: 0.97rem; }

.tabs { display: flex; flex-wrap: wrap; gap: 0.5rem; margin-top: 1.75rem; }
.tab {
  appearance: none;
  display: inline-flex;
  align-items: center;
  gap: 0.5rem;
  padding: 0.6rem 1.1rem;
  border: 1px solid var(--line);
  border-radius: 999px;
  background: var(--card);
  color: var(--ink-soft);
  font: inherit;
  font-weight: 700;
  cursor: pointer;
  transition: border-color 0.2s ease, background 0.2s ease, color 0.2s ease;
}
.tab:hover { border-color: rgba(63, 111, 42, 0.4); }
.tab.is-on { background: var(--accent); border-color: var(--accent); color: #fff; }
.who {
  margin-top: 2rem;
  display: grid;
  gap: 2.5rem;
  grid-template-columns: 1fr 1.4fr;
  align-items: start;
}
.who__text h3 { font-size: clamp(1.25rem, 2.6vw, 1.6rem); margin-bottom: 1rem; }
.who__text .list-check { margin-bottom: 1.5rem; }
.who__text .list-check li { color: var(--ink-soft); }
.who__day { color: var(--ink-soft); margin: 0 0 1.5rem; }

.own { display: grid; gap: 2.5rem; grid-template-columns: 1fr 1.1fr; align-items: center; }
@media (max-width: 900px) {
  .who, .own { grid-template-columns: 1fr; }
  .hero__bubble { max-width: 13.5rem; }
}
</style>
