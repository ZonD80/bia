<template>
  <main>
    <section class="page-hero">
      <div class="container">
        <nav class="crumbs" aria-label="Breadcrumb">
          <NuxtLink to="/">Home</NuxtLink><span aria-hidden="true">/</span><span>FAQ</span>
        </nav>
        <span class="eyebrow">Questions people ask</span>
        <h1>Frequently asked questions</h1>
      </div>
    </section>

    <section class="section">
      <div class="narrow">
        <div v-for="s in sections" :key="s.title" class="faqs">
          <h2>{{ s.title }}</h2>
          <details v-for="q in s.items" :key="q.q" class="faq">
            <summary>{{ q.q }}</summary>
            <p>{{ q.a }}</p>
          </details>
        </div>
      </div>
    </section>

    <CtaBand title="Still curious?" text="The best way to understand bia is to wake one up. It takes a USB stick and twenty minutes." />
  </main>
</template>

<script setup lang="ts">
useSeoMeta({
  title: 'FAQ — questions about bia',
  description: 'Answers about bia: what it is, what it costs, privacy, the internet, old computers, languages, messengers and using it responsibly.',
})

const sections = [
  {
    title: 'The basics',
    items: [
      { q: 'What is bia, in one sentence?', a: 'A free program you put on a USB stick that turns any computer into a helpful AI personality — it talks with people on WhatsApp, Telegram and phone calls, remembers, uses the computer and the web, and can move to new machines.' },
      { q: 'Is it a robot?', a: 'It has no body — but it has a computer, a voice, accounts and a memory. Think of it as a very capable assistant who lives inside a computer and never sleeps.' },
      { q: 'Do I need to know how to program?', a: 'No. You speak to bia in your own words, in your own language, and it does the technical work itself. It asks before anything risky.' },
      { q: 'What does it cost?', a: 'bia itself is free software. For thinking, bia uses a cloud brain, and the AI provider charges you directly for what you use. At the moment the cheapest way is DeepSeek V4.1. A brain on the stick has no usage fees, but the computer it needs costs far more than years of cloud use.' },
    ],
  },
  {
    title: 'Computers and internet',
    items: [
      { q: 'Will it work on my old laptop?', a: 'Very likely. bia runs on most PCs and laptops of the last decade and on many ARM mini-PCs. Ordinary and older machines use the cloud brain. Running the brain on the stick, without internet, needs a high-end graphics card or unified memory with at least 64 GB of video memory (VRAM).' },
      { q: 'Does it delete Windows?', a: 'No. Starting from the stick changes nothing on the computer. If you later want bia on the computer’s disk, you can choose an empty disk or install it beside Windows.' },
      { q: 'Does it need the internet?', a: 'Normally, yes: messengers and the cloud brain need the internet. If you need bia to work with no internet at all, the brain on the stick is an option — it needs a computer with a high-end graphics card or unified memory with at least 64 GB of video memory (VRAM).' },
      { q: 'Do I need a screen?', a: 'Only for the first minutes. After that you can reach bia from your phone through its private app, and the computer can live in a cupboard.' },
    ],
  },
  {
    title: 'Talking and languages',
    items: [
      { q: 'Which languages does it speak?', a: 'bia understands speech in about a hundred languages and can speak over forty of them out loud. When it meets a new language, it learns it by itself and tells you.' },
      { q: 'Can it answer phone calls?', a: 'Yes — Telegram and WhatsApp voice calls, plus live calls in its own app. After each call, it posts a written summary of what was said.' },
      { q: 'Does it use my Telegram and WhatsApp?', a: 'It can use an account you give it — your own, or one created for bia. On WhatsApp it joins as a linked device, the same way WhatsApp Web does.' },
    ],
  },
  {
    title: 'Privacy and safety',
    items: [
      { q: 'Where are my conversations stored?', a: 'On bia’s own disk, on the computer you control. Nothing is uploaded to a bia company — there isn’t one collecting your data.' },
      { q: 'Who can talk to bia?', a: 'Anyone you let reach it on messengers can chat with it. Only people you trust fully can ask it to do serious things on the computer, and its private app requires a personal key that you hand out and can take back.' },
      { q: 'Is the cloud brain private?', a: 'When you use a cloud brain, what bia is thinking about is sent to that provider, under their privacy terms. If your work must never leave the building, the brain on the stick is an option on a powerful computer.' },
      { q: 'What if the computer is stolen?', a: 'bia’s disk holds its memories and vault, so treat its computer like your phone: keep it somewhere safe, and keep a backup of bia in a safe place.' },
    ],
  },
  {
    title: 'Using it responsibly',
    items: [
      { q: 'Can bia pretend to be me?', a: 'It can write from an account you give it — which is why we ask you to use it honestly. Let people know when they are talking to an assistant where it matters, and never use bia to deceive or harass anyone.' },
      { q: 'Does it follow the messengers’ rules?', a: 'You are responsible for using bia within the terms of each service it joins. bia is designed to be a well-behaved participant: it answers, it does not spam.' },
      { q: 'What if someone tries to trick it?', a: 'bia has trust levels and good judgement: strangers cannot order it around, and it reports attempts to pressure it instead of obeying.' },
    ],
  },
]
</script>

<style scoped>
.faqs { margin-bottom: 2.5rem; }
.faqs h2 { font-size: 1.5rem; margin-bottom: 0.8rem; }
.faq {
  border: 1px solid var(--line);
  border-radius: 0.8rem;
  background: var(--card);
  margin-bottom: 0.6rem;
  padding: 0 1.1rem;
}
.faq summary {
  cursor: pointer;
  padding: 1rem 0;
  font-weight: 700;
  list-style: none;
  display: flex;
  justify-content: space-between;
  gap: 1rem;
}
.faq summary::-webkit-details-marker { display: none; }
.faq summary::after {
  content: '+';
  font-size: 1.3rem;
  line-height: 1;
  color: var(--accent);
}
.faq[open] summary::after { content: '–'; }
.faq p { color: var(--ink-soft); margin: 0 0 1rem; }
</style>
