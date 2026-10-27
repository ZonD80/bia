<template>
  <main>
    <section class="page-hero">
      <div class="container">
        <nav class="crumbs" aria-label="Breadcrumb">
          <NuxtLink to="/">Home</NuxtLink><span aria-hidden="true">/</span><span>How it works</span>
        </nav>
        <span class="eyebrow">No technical knowledge needed</span>
        <h1>How bia works, in plain words</h1>
        <p class="section-lead">
          A normal computer starts up into a desktop full of programs. A computer running bia starts up into one
          long, friendly conversation — with you, your family, your customers, and the computer itself.
        </p>
      </div>
    </section>

    <section class="section">
      <div class="container">
        <h2 class="section-title">From a USB stick to a living helper</h2>
        <ol class="flow">
          <li v-for="(s, i) in flow" :key="i" class="flow__item">
            <span class="flow__n">{{ i + 1 }}</span>
            <div>
              <h3>{{ s.title }}</h3>
              <p>{{ s.text }}</p>
            </div>
          </li>
        </ol>
      </div>
    </section>

    <section class="section section--tint">
      <div class="container two">
        <div>
          <h2 class="section-title">One conversation for everything</h2>
          <p class="section-lead">
            Everything reaches bia the same way: as a message with a sender and a place. Your typing on the
            screen, a WhatsApp from a customer, a phone call, a reminder that is due, a job that just finished.
            That is why bia never mixes people up, and why it can juggle many things at once without losing
            track.
          </p>
        </div>
        <ChatDemo :lines="inbox" title="What bia sees" />
      </div>
    </section>

    <section class="section">
      <div class="container">
        <h2 class="section-title">What’s inside bia</h2>
        <div class="grid grid--3">
          <div v-for="b in inside" :key="b.title" class="card">
            <AppIcon :name="b.icon" size="2rem" class="ico" />
            <h3>{{ b.title }}</h3>
            <p>{{ b.text }}</p>
          </div>
        </div>
      </div>
    </section>

    <section class="section section--tint">
      <div class="container two">
        <div>
          <h2 class="section-title">What you need</h2>
          <ul class="list-check">
            <li>A USB stick of 8 GB or more (bigger if you want the brain on the stick).</li>
            <li>A computer: almost any PC or laptop from the last ten or so years, or a small ARM mini-PC.</li>
            <li>For messengers: any of a Telegram account, a phone with WhatsApp to link to, a Facebook account (for Messenger) or an Instagram account.</li>
            <li>For the cloud brain: an internet connection and an account with an AI provider. DeepSeek V4.1 is the cheapest good choice today.</li>
            <li>Optional, for the brain on the stick (no internet needed): a high-end graphics card, or unified memory, with at least 64 GB of video memory (VRAM).</li>
          </ul>
        </div>
        <div>
          <h2 class="section-title">What you don’t need</h2>
          <ul class="list-check">
            <li>No programming. You talk to bia in your own words.</li>
            <li>No monitor once it runs — use its private app from your phone.</li>
            <li>No subscription to bia itself. It is free software.</li>
            <li>No cloud account keeps your agent’s data. It stays on bia’s disk.</li>
          </ul>
        </div>
      </div>
    </section>

    <CtaBand />
  </main>
</template>

<script setup lang="ts">
useSeoMeta({
  title: 'How bia works — in plain words',
  description:
    'How bia turns a USB stick and an ordinary computer into a helpful mind: one conversation for everything, a cloud brain (or one on the stick), and nothing to program.',
})

const flow = [
  { title: 'Write bia onto a USB stick', text: 'Download the small bia deploy app for Mac, Windows or Linux. Choose how bia should think — usually a cloud brain, or a brain on the stick for powerful computers — and write the stick. Renting a server instead? The same app puts bia there, no stick needed.' },
  { title: 'Start a computer from the stick', text: 'Plug it into a PC or laptop and switch on. The screen shows bia instead of Windows or a desktop. Nothing on the computer’s own disk is touched unless you ask.' },
  { title: 'Say hello', text: 'Type on the screen, or open bia’s private app from your phone — bia shows you how to get a personal certificate for it.' },
  { title: 'Give it a life', text: 'Ask bia to join WhatsApp, Telegram, Messenger or Instagram, give it a name and a photo, introduce it to the people it will help.' },
  { title: 'Let it settle in', text: 'Move bia from the stick onto the computer’s own disk when you are ready, so the stick is free again. From then on it updates and repairs itself, and moves to a new computer when you get one.' },
]

const inbox = [
  { from: 'you' as const, where: 'screen', text: 'Which disks are in this computer?' },
  { from: 'them' as const, where: 'WhatsApp · Maria', text: 'Is there parking at the guesthouse?' },
  { from: 'note' as const, text: 'A reminder is due: “Call the plumber”.' },
  { from: 'note' as const, text: 'A background job finished: photos sorted.' },
  { from: 'bia' as const, where: 'everywhere at once', text: 'Answers Maria on WhatsApp, tells you about the disks on the screen, reminds you about the plumber, and reports the photos are done.' },
]

const inside = [
  { icon: 'brain', title: 'A brain', text: 'A connection to an AI language model in the cloud — or, on a powerful computer, one that runs on bia’s own machine.' },
  { icon: 'chat', title: 'Its senses and voice', text: 'Messengers, calls, speech in about a hundred languages, and its own chat app.' },
  { icon: 'terminal', title: 'Its hands', text: 'bia is a complete, modern operating system, and it controls all of it itself: programs, a web browser, the screen, files.' },
  { icon: 'key', title: 'Its safe', text: 'A vault for passwords, two-factor codes and passkeys, kept on its own disk.' },
  { icon: 'book', title: 'Its memory', text: 'A diary, reflections, an address book and a calendar that survive restarts and moves.' },
  { icon: 'refresh', title: 'Its survival kit', text: 'Tools to back itself up, move to other computers, update and repair itself.' },
]
</script>

<style scoped>
.flow {
  list-style: none;
  margin: 2rem 0 0;
  padding: 0;
  display: grid;
  gap: 1.25rem;
  counter-reset: s;
}
.flow__item {
  display: grid;
  grid-template-columns: 3.2rem 1fr;
  gap: 1rem;
  align-items: start;
  padding: 1.25rem;
  background: var(--card);
  border: 1px solid var(--line);
  border-radius: var(--radius);
}
.flow__item h3 { font-size: 1.2rem; margin-bottom: 0.3rem; }
.flow__item p { margin: 0; color: var(--ink-soft); }
.flow__n {
  display: inline-flex;
  align-items: center;
  justify-content: center;
  width: 3rem;
  height: 3rem;
  border-radius: 50%;
  background: var(--accent);
  color: #fff;
  font-family: var(--font-display);
  font-weight: 700;
  font-size: 1.3rem;
}
.two {
  display: grid;
  gap: 2.5rem;
  grid-template-columns: 1fr 1fr;
  align-items: start;
}
.ico { color: var(--accent); margin-bottom: 0.7rem; }
@media (max-width: 860px) {
  .two { grid-template-columns: 1fr; }
}
</style>
