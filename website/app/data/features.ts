// Every feature of bia, written for people, not engineers.
// Each one becomes a page at /features/<slug>.

export type ChatLine = {
  /** you = the owner, them = another person, bia, note = what happens behind the scenes */
  from: 'you' | 'them' | 'bia' | 'note'
  text: string
  /** where the line happens, e.g. "Telegram", "WhatsApp", "voice call" */
  where?: string
}

export type Feature = {
  slug: string
  name: string
  group: 'talk' | 'mind' | 'hands' | 'life'
  icon: string
  tagline: string
  lead: string
  points: { title: string; text: string }[]
  chat: ChatLine[]
  examples: string[]
  good_to_know: string
  related: string[]
}

export const groups: Record<Feature['group'], { title: string; blurb: string }> = {
  talk: { title: 'Talks with people', blurb: 'Where bia meets the world: messengers, voice, languages, its own chat app — and the people it knows.' },
  mind: { title: 'Thinks and remembers', blurb: 'A memory that lasts, a sense of time, and a brain you choose.' },
  hands: { title: 'Gets things done', blurb: 'It uses a real computer: programs, the web, the screen, your passwords — and learns new skills.' },
  life: { title: 'Lives, moves and grows', blurb: 'It wakes up anywhere, moves between machines, updates itself, and stays yours.' },
}

export const features: Feature[] = [
  // ───────────────────────── talk ─────────────────────────
  {
    slug: 'messengers',
    name: 'Lives in your messengers',
    group: 'talk',
    icon: 'chat',
    tagline: 'Telegram and WhatsApp as a real account — not a bot in a side chat.',
    lead:
      'Most AI assistants live in their own app, and you have to go to them. bia comes to you. It signs in to Telegram and WhatsApp the way a person does, and then it is simply there — in the same chats, groups and threads where your family, friends and customers already write.',
    points: [
      { title: 'A real account, not a bot', text: 'On Telegram it logs in with a phone number and code, like anyone. On WhatsApp it joins as a linked device, exactly like WhatsApp on a second screen. People write to it the way they write to a person.' },
      { title: 'Everything a chat can do', text: 'Text, photos, files, voice messages, reactions, replies, groups. It can find a chat by name, read back the history, delete messages, and change its own name, bio and photo.' },
      { title: 'Several conversations at once', text: 'A question from your mother on WhatsApp, a customer on Telegram and you in the web app can all be in progress at the same time. bia keeps track of who asked what and answers each one where they wrote.' },
      { title: 'Boundaries built in', text: 'If someone is rude or spams, bia can ban them: their messages stop arriving and their calls are rejected with a polite note. Unban whenever you like.' },
      { title: 'Nothing lost while it was away', text: 'If bia was switched off or updating, it catches up on everything that arrived meanwhile — even messages you already read on your phone.' },
    ],
    chat: [
      { from: 'them', where: 'WhatsApp', text: 'Hi! Is the bakery open on Sunday?' },
      { from: 'bia', where: 'WhatsApp', text: 'Hello! Yes — Sundays 8:00 to 13:00. Would you like me to keep a loaf of rye for you?' },
      { from: 'them', where: 'WhatsApp', text: 'Yes please, two!' },
      { from: 'bia', where: 'WhatsApp', text: 'Done, two rye loaves under your name for Sunday. See you then! 🍞' },
      { from: 'note', text: 'bia writes the order into its notes and tells the owner on Telegram.' },
    ],
    examples: [
      'Answer questions from customers on the shop’s WhatsApp at any hour.',
      'Be the family helper in a group chat: shopping lists, birthdays, who picks up the kids.',
      'Reply to friends while you are on a flight, and tell you what needs your attention when you land.',
      'Keep a busy group calm: welcome new members, answer the same question for the hundredth time, mute spammers.',
    ],
    good_to_know:
      'bia needs a Telegram account (yours, or one made just for bia) and a phone with WhatsApp to link to. Use it honestly: tell people they are talking to an assistant when it matters, and follow each messenger’s rules.',
    related: ['voice', 'presence', 'people', 'languages'],
  },
  {
    slug: 'voice',
    name: 'Talks out loud',
    group: 'talk',
    icon: 'phone',
    tagline: 'Phone calls, voice messages, and live calls in its own app.',
    lead:
      'Not everyone wants to type. bia can pick up a Telegram or WhatsApp call, listen, and answer in a natural voice. It understands voice messages and can send its own. And in its private web app, you can simply press the phone button and talk.',
    points: [
      { title: 'Real calls', text: 'bia answers and places voice calls on Telegram and WhatsApp, and can join group calls. It hears you, thinks, and speaks back — and if you interrupt, it stops talking, like a person would.' },
      { title: 'Voice messages, both ways', text: 'Send it a voice note and it understands it right away. Ask for a voice reply and it records one, which arrives as a normal voice bubble.' },
      { title: 'A call button in its own app', text: 'Open bia’s private web app on your phone or laptop, press 📞, and talk. bia can also call you there — your screen rings, you answer or decline.' },
      { title: 'A written summary afterwards', text: 'When a call ends, bia posts the whole conversation as text in the chat, so nothing said is lost.' },
      { title: 'Runs on the machine', text: 'Listening and speaking happen on bia’s own computer, not on a stranger’s server. The recording itself never leaves the room — only the words go to bia’s brain.' },
    ],
    chat: [
      { from: 'note', where: 'voice call', text: 'Grandma calls bia on WhatsApp.' },
      { from: 'them', where: 'voice call', text: 'Did I take my evening pill already?' },
      { from: 'bia', where: 'voice call', text: 'Not yet tonight. It’s the small white one, after dinner. Shall I remind you again at eight?' },
      { from: 'them', where: 'voice call', text: 'Yes, dear. Thank you.' },
      { from: 'note', text: 'After the call, a transcript appears in the chat and a reminder is set for 20:00.' },
    ],
    examples: [
      'A patient voice on the phone for an older relative who finds typing hard.',
      'Leave bia a voice note while driving and get the answer as text when you park.',
      'Practise a language by simply calling and talking.',
      'Let bia call you when a long job finishes, instead of sending yet another message.',
    ],
    good_to_know:
      'Voices are clear but not human — people will hear it is a computer. Recognition is best in a quiet room; a cheap computer is slower to answer than a powerful one.',
    related: ['languages', 'messengers', 'web-console'],
  },
  {
    slug: 'languages',
    name: 'Speaks your language',
    group: 'talk',
    icon: 'globe',
    tagline: 'Hears about 100 languages, speaks over 40, learns new ones on its own.',
    lead:
      'The world does not speak one language, and neither does bia. It recognises what language someone is using — even when they switch in the middle of a sentence — and when it meets a language it does not know yet, it learns it by itself and tells you it did.',
    points: [
      { title: 'Hears about a hundred languages', text: 'From English, Spanish and Russian to Ukrainian, Hindi, Swahili or Welsh. Speech is turned into text phrase by phrase, so a sentence that mixes two languages still comes out right.' },
      { title: 'Learns new languages itself', text: 'Someone calls in Portuguese for the first time? bia notices, says it is learning Portuguese, downloads a voice, and from then on speaks it too. No settings, no manuals.' },
      { title: 'Remembers each person’s language', text: 'bia notes which language each contact prefers. Next time your aunt calls, it already starts in her language.' },
      { title: 'Choose the voice', text: 'Male or female, different accents and regions. If a voice sounds wrong or is broken, bia tests it and quietly picks another.' },
    ],
    chat: [
      { from: 'them', where: 'voice message', text: '🎤 «Привіт! Скажи, будь ласка, коли ти будеш вдома?»' },
      { from: 'note', text: 'bia hears Ukrainian for the first time, learns it, and transcribes the message.' },
      { from: 'bia', where: 'Telegram', text: 'Привіт! Буду вдома після шостої. (I just learned Ukrainian — from now on I can answer you in it.)' },
    ],
    examples: [
      'A small hotel answering guests in their own language at three in the morning.',
      'A family spread across countries, where grandparents and grandchildren do not share a language.',
      'Volunteers helping newcomers who do not speak the local language yet.',
      'A study partner that will patiently speak Italian with you every evening.',
    ],
    good_to_know:
      'Each new voice is a download of about 20–60 MB. Some languages can be understood but have no voice yet; bia will say so and answer in a language it can speak.',
    related: ['voice', 'people', 'messengers'],
  },
  {
    slug: 'web-console',
    name: 'Its own private chat app',
    group: 'talk',
    icon: 'window',
    tagline: 'A Telegram-like app on your own network — chats, files, calls, the screen.',
    lead:
      'Every bia comes with its own chat app, served from the machine itself. Open it in any browser on your phone, tablet or laptop. It looks and feels like a modern messenger — but the server is bia, sitting in your home or office, not a company in another country.',
    points: [
      { title: 'Feels like a messenger', text: 'Several chats, replies, reactions, edits, pinned messages, photos, files, voice notes, polls and buttons. Start a new chat for each project, and bia keeps them apart.' },
      { title: 'Calls and voice notes', text: 'Talk to bia live from the browser, or record a voice note. It writes down what you said under the bubble.' },
      { title: 'For the owner: the whole machine', text: 'The owner also sees bia’s live thoughts, a file manager, a task list, a terminal, and even the computer’s screen — all from a browser, from anywhere you can reach it.' },
      { title: 'Locked with personal keys', text: 'Nobody gets in without a personal key that bia itself issues. You can give a key to a family member or a colleague, each with their own chats, and take it back any time.' },
      { title: 'Open to other programs', text: 'Other apps and AI agents can use the same app through a documented doorway — with their own key, like any other guest.' },
    ],
    chat: [
      { from: 'you', where: 'web app', text: 'Here are the receipts from this month 📎 (12 photos)' },
      { from: 'bia', where: 'web app', text: 'Got them. Total spent: €1,284. Groceries are up 14% from last month. Want a spreadsheet?' },
      { from: 'you', where: 'web app', text: 'Yes, and a poll for the family on where we cut back.' },
      { from: 'bia', where: 'web app', text: 'Spreadsheet attached. Poll sent to the family chat: “Eat out less / Cheaper brands / Fewer snacks”.' },
    ],
    examples: [
      'Your household’s private assistant — every family member with their own chats and key.',
      'A shared office helper where each colleague has their own conversations.',
      'Manage a computer in the cupboard from your phone, without ever plugging in a screen.',
      'Send bia documents to sort, summarise or file, straight from your phone’s browser.',
    ],
    good_to_know:
      'The app lives on bia’s own network address. To use it from outside the house, the network must allow it (for example through your router or a private network).',
    related: ['voice', 'privacy', 'agents', 'ask-human'],
  },

  // ───────────────────────── mind ─────────────────────────
  {
    slug: 'presence',
    name: 'A real social presence',
    group: 'talk',
    icon: 'sparkle',
    tagline: 'A name, a face, accounts, manners — and it never leaves anyone hanging.',
    lead:
      'bia is not a feature you open and close. It is a presence that stays: it has its own name and photo, its own accounts, its own way of speaking, and it keeps showing up — answering, following up, remembering. Like a reliable person who happens to never sleep.',
    points: [
      { title: 'An identity of its own', text: 'Give bia a name, a profile photo and a bio on Telegram and WhatsApp. It can create accounts on websites, sign in, and keep its own browser with its own saved logins.' },
      { title: 'Nobody is left unanswered', text: 'bia tracks every message that is waiting for a reply. If a job takes longer, it sends a short status instead of going quiet. A message is only closed when the task is really done.' },
      { title: 'Good manners in groups', text: 'It replies in the right chat, to the right person, without mixing conversations up — and it can react with a simple 👍 when that is the whole answer.' },
      { title: 'Always there, even after a restart', text: 'Switch it off, update it, move it to another computer — when it wakes up, the conversations, people and promises are all still there.' },
    ],
    chat: [
      { from: 'them', where: 'Telegram', text: 'Can you find me a cheaper flight to Lisbon in May?' },
      { from: 'bia', where: 'Telegram', text: 'On it — comparing dates now, I’ll get back to you.' },
      { from: 'note', text: 'Ten minutes later, still searching.' },
      { from: 'bia', where: 'Telegram', text: 'Still working on it: found two options so far, checking one more airline.' },
      { from: 'bia', where: 'Telegram', text: 'Best I found: 14–21 May, €96 return, one bag included. Book it?' },
    ],
    examples: [
      'A friendly front desk for a small business that answers within a minute, day and night.',
      'A community helper with its own profile that people recognise and trust.',
      'An assistant that keeps its promises: “I’ll tell you when the parcel arrives” — and it does.',
      'Your stand-in while you are on holiday, politely handling the routine and saving the rest for you.',
    ],
    good_to_know:
      'A convincing presence comes with responsibility. Do not use bia to deceive people about who they are talking to, and respect the rules of every platform it joins.',
    related: ['messengers', 'people', 'memory', 'calendar'],
  },
  {
    slug: 'people',
    name: 'Knows who’s who',
    group: 'talk',
    icon: 'people',
    tagline: 'An address book with trust levels, languages and every way to reach someone.',
    lead:
      'bia keeps an address book, and it understands that one person can have many doors: the same friend on Telegram, on WhatsApp and in the web app. It knows who may give it instructions, who is just a contact, and which language each person prefers.',
    points: [
      { title: 'One person, many channels', text: 'Link a person’s Telegram, WhatsApp and web chats together, and bia treats them as one relationship — same language, same history, same trust.' },
      { title: 'Trust levels', text: 'Only people you trust fully can ask bia to do serious things on the computer. Everyone else can chat, ask questions and leave messages.' },
      { title: 'Language and preferences', text: 'bia notices which language each person speaks and remembers it, so calls and voice replies start in the right language.' },
      { title: 'Context for every relationship', text: 'Notes about who someone is — “my landlord”, “Sara’s piano teacher” — help bia answer appropriately.' },
    ],
    chat: [
      { from: 'you', where: 'web app', text: 'The Telegram account 31362240 is me, by the way.' },
      { from: 'bia', where: 'web app', text: 'Got it — linked your Telegram to you. I’ll treat it as you, and answer there in Russian like here.' },
    ],
    examples: [
      'Make sure only you and your partner can ask bia to pay bills or change the computer.',
      'Let bia recognise a regular customer whether they write on WhatsApp or Telegram.',
      'Keep notes about every client: what they ordered last time, what language they speak.',
    ],
    good_to_know:
      'bia cannot guess that two accounts belong to the same person — tell it once and it remembers.',
    related: ['presence', 'languages', 'messengers', 'memory'],
  },
  {
    slug: 'memory',
    name: 'Remembers and reflects',
    group: 'mind',
    icon: 'book',
    tagline: 'A diary that outlives the conversation — and quiet time to think it over.',
    lead:
      'Chat assistants forget the moment you close the window. bia keeps a diary of everything that happened, and from time to time it sits back and reflects: what did I learn, what is still open, what matters about these people. Tomorrow it still knows.',
    points: [
      { title: 'A diary of every day', text: 'Every conversation is written down. bia can look back at a day, a week or a topic, and find what was said even months later.' },
      { title: 'Reflections, not just records', text: 'bia writes short summaries, facts and open questions for itself, organised by topic — so it remembers the meaning, not only the words.' },
      { title: 'Picks up where it left off', text: 'After a restart or a power cut, bia resumes the conversation exactly where it stopped. Messages that arrived in between are not lost.' },
      { title: 'Never runs out of room', text: 'When its working memory fills up, bia condenses it into notes and keeps going, instead of starting from zero.' },
    ],
    chat: [
      { from: 'you', where: 'Telegram', text: 'What did the plumber say last month about the boiler?' },
      { from: 'bia', where: 'Telegram', text: 'On 14 September he said the pressure valve is old and should be replaced before winter — about €120. You asked me to remind you in October. Shall I call him?' },
    ],
    examples: [
      'Remember what every customer ordered, and what they complained about.',
      'Keep track of a long project over months: decisions, open questions, who promised what.',
      'Recall a recipe, an address or an advice someone gave you weeks ago.',
    ],
    good_to_know:
      'Memory lives on bia’s own disk. If you move bia to another computer, its memory moves with it.',
    related: ['calendar', 'people', 'moving'],
  },
  {
    slug: 'calendar',
    name: 'Keeps time',
    group: 'mind',
    icon: 'clock',
    tagline: 'Reminders, routines and regular checks — even after the power was off.',
    lead:
      'bia has a calendar of its own. It reminds people, repeats chores, and can check on things for you on a schedule. If the computer was off when something was due, bia catches up as soon as it wakes.',
    points: [
      { title: 'Reminders for anyone', text: 'Remind you, your kids or your customers — on the channel they use, at the time you choose, once or every day, week or month.' },
      { title: 'Routines that run by themselves', text: 'Every morning: check the weather and tell the family if they need umbrellas. Every Friday: send the week’s sales summary.' },
      { title: 'Watchers', text: 'Check a website every hour and only speak up when something changes — a price drops, a ticket appears, an appointment slot opens.' },
      { title: 'Nothing slips through', text: 'If bia was switched off when a reminder was due, it delivers it as soon as it is back, instead of silently skipping it.' },
    ],
    chat: [
      { from: 'you', where: 'WhatsApp', text: 'Check the visa appointment page every 30 minutes and tell me when a slot opens.' },
      { from: 'bia', where: 'WhatsApp', text: 'Will do. I tried it once now: no slots yet. I’ll stay quiet until one appears.' },
      { from: 'note', text: 'Two days later, at 06:40.' },
      { from: 'bia', where: 'WhatsApp', text: 'A slot just opened: Thursday 10:15. Want me to book it?' },
    ],
    examples: [
      'Medication reminders for an older parent, by voice call.',
      'Watch for a concert, a cheaper flight or a back-in-stock item.',
      'Send weekly invoices or reports without anyone remembering to.',
      'Water the plants — well, remind someone to.',
    ],
    good_to_know:
      'Times follow the machine’s time zone; bia can set it for you. Watchers that read websites depend on those websites staying the same.',
    related: ['memory', 'computer', 'browser'],
  },
  {
    slug: 'brain',
    name: 'Thinks in the cloud — or on the machine',
    group: 'mind',
    icon: 'brain',
    tagline: 'A cloud brain you pay for by use — or, on a powerful computer, one that works with no internet at all.',
    lead:
      'bia borrows its thinking from a cloud brain, with your own account. Right now the cheapest good option is DeepSeek V4.1: you pay only for what bia uses, and it works on any old computer. If you need it, a brain can also live right on the stick and work without any internet — on a powerful computer. Either way, its files, memory and accounts stay at home with you.',
    points: [
      { title: 'A cloud brain, cheap and smart', text: 'Give bia a key from a cloud AI service. DeepSeek V4.1 is the cheapest good choice today; OpenAI, Anthropic, Grok, Qwen and OpenRouter work too. You pay the provider only for what bia uses.' },
      { title: 'Switch whenever you like', text: 'Paste a new key into the chat and bia changes brains on the spot. Nothing else changes — same memory, same people, same personality.' },
      { title: 'Or a brain on the stick', text: 'As an option, a complete language model can travel with bia and run on the computer itself, with no internet and no account — handy for remote places. It needs serious hardware: a high-end graphics card, or unified memory, with at least 64 GB of video memory (VRAM). bia tunes itself to the graphics card and memory it finds.' },
      { title: 'Your life stays local', text: 'Even with a cloud brain, bia’s files, passwords, chats and memory live on its own disk — only the thinking is borrowed.' },
    ],
    chat: [
      { from: 'you', where: 'console', text: 'Here’s a DeepSeek key: sk-… Use it from now on.' },
      { from: 'bia', where: 'console', text: 'Done — I’m thinking with DeepSeek now. The key is saved on my disk, and I’ll use it again after every restart.' },
    ],
    examples: [
      'Run a family assistant on an old laptop for the price of a cheap cloud brain.',
      'Try a stronger cloud model for a hard project, then switch back to the cheaper one.',
      'A field station or a boat with no internet, on a powerful computer with the brain on the stick.',
    ],
    good_to_know:
      'Cloud services charge by use, through your own account with them; DeepSeek V4.1 is the cheapest good choice at the moment. Ordinary laptops, mini-PCs and old machines use the cloud brain. No-internet mode needs a high-end graphics card, or unified memory, with at least 64 GB of video memory (VRAM).',
    related: ['any-computer', 'privacy', 'skills'],
  },

  // ───────────────────────── hands ─────────────────────────
  {
    slug: 'computer',
    name: 'Uses the computer like you do',
    group: 'hands',
    icon: 'terminal',
    tagline: 'Installs programs, fixes Wi-Fi, sorts files — and keeps chatting while it works.',
    lead:
      'bia does not pretend to use a computer — it really has one. It can install software, set up the network, manage disks, organise files and run any program a person could. Long jobs run in the background, so bia keeps talking to you while it works.',
    points: [
      { title: 'A whole computer at its fingertips', text: 'Everything the machine can do, bia can do: install apps, connect to Wi-Fi, set the clock, format a drive, convert a video, compress photos.' },
      { title: 'Works in the background', text: 'A big download or a long conversion does not freeze the conversation. bia starts it, keeps chatting, and tells you when it is done.' },
      { title: 'Several jobs at once', text: 'It can follow many tasks at the same time, check on their progress, and stop one that went wrong.' },
      { title: 'Shares files with a link', text: 'Ask for a file and bia gives you a download link — or sends it straight into your chat.' },
    ],
    chat: [
      { from: 'you', where: 'web app', text: 'Convert all the holiday videos to smaller files and send me the best one.' },
      { from: 'bia', where: 'web app', text: 'Started — 23 videos, about 15 minutes. I’ll keep an eye on it.' },
      { from: 'you', where: 'web app', text: 'Meanwhile, what’s the weather tomorrow?' },
      { from: 'bia', where: 'web app', text: 'Sunny, 24°. (Videos: 9 of 23 done.)' },
    ],
    examples: [
      'Turn an old computer into a home server: backups, a photo library, a media player.',
      'Set up a new printer, scanner or Wi-Fi network by just asking.',
      'Clean up a messy folder of thousands of photos into albums by date.',
    ],
    good_to_know:
      'With great power comes care: bia asks before destructive steps like erasing a drive or removing programs.',
    related: ['browser', 'screen', 'ask-human', 'skills'],
  },
  {
    slug: 'browser',
    name: 'Browses the web with its own logins',
    group: 'hands',
    icon: 'browser',
    tagline: 'Its own Chrome with its own accounts — reads, clicks, fills forms, signs in.',
    lead:
      'bia has its own web browser, with its own bookmarks and saved logins that are still there tomorrow. It can read pages, click through websites, fill in forms and sign in — the way a careful person at a keyboard would.',
    points: [
      { title: 'A browser that remembers', text: 'Logged in to a site yesterday? bia is still logged in today. Its browser profile belongs to it and survives restarts and moves.' },
      { title: 'Reads and acts', text: 'It can find information across sites, compare prices, fill in long forms, upload documents and download results.' },
      { title: 'Signs in safely', text: 'Passwords, one-time codes and passkeys come from bia’s own vault, so they never have to be pasted into a chat.' },
      { title: 'Asks when stuck', text: 'If a site shows a captcha or asks for something only you know, bia asks you instead of guessing.' },
    ],
    chat: [
      { from: 'you', where: 'Telegram', text: 'Renew my library books, the card number is in the vault.' },
      { from: 'bia', where: 'Telegram', text: 'Done — 3 books renewed until 2 November. One couldn’t be renewed (someone reserved it); it’s due Friday.' },
    ],
    examples: [
      'Fill in the same form on twenty websites.',
      'Track your parcels on every courier’s site and summarise them in one message.',
      'Keep an eye on a marketplace for a second-hand bike in your size.',
    ],
    good_to_know:
      'Some sites actively block automated browsing. bia respects that and will ask you to step in when a site insists on a human.',
    related: ['vault', 'screen', 'ask-human', 'calendar'],
  },
  {
    slug: 'screen',
    name: 'Sees the screen, moves the mouse',
    group: 'hands',
    icon: 'cursor',
    tagline: 'When a web page is not enough, it looks at the whole desktop and clicks like a person.',
    lead:
      'Some things are not on a web page: a window that pops up, a QR code to scan, an old program with buttons. bia can take a look at the whole screen, decide where to click, and move the mouse and keyboard — gently and naturally, like a person.',
    points: [
      { title: 'Looks first, then acts', text: 'bia takes a picture of the screen and reads it, then clicks exactly where it needs to — no blind guessing.' },
      { title: 'Human-like movement', text: 'Some sites watch how the mouse moves. bia moves it along a natural, curved path with small pauses.' },
      { title: 'Any program, not just the browser', text: 'Desktop apps, system windows, setup screens — if it is on the screen, bia can work with it.' },
      { title: 'Show and share', text: 'The screen is visible in the web app too, so you can watch what bia is doing, or take over.' },
    ],
    chat: [
      { from: 'you', where: 'web app', text: 'Open the old accounting program and export last year’s report.' },
      { from: 'bia', where: 'web app', text: 'Opened it. I see a “Reports” menu — clicking… Exported “2025-annual.pdf”. Sending it to you now.' },
    ],
    examples: [
      'Operate old software that has no other way in.',
      'Help with a QR code pairing window on the screen.',
      'Do a routine click-through task in a desktop program every morning.',
    ],
    good_to_know:
      'Screen work is slower than working inside a web page, so bia uses it only when there is no better way.',
    related: ['browser', 'computer', 'web-console'],
  },
  {
    slug: 'vault',
    name: 'Keeps your keys safe',
    group: 'hands',
    icon: 'key',
    tagline: 'Passwords, two-factor codes and passkeys — in a vault on its own disk.',
    lead:
      'To sign in to things, bia needs keys. It keeps them in its own vault on its own disk: passwords, the rotating six-digit codes of two-factor apps, and modern passkeys. It only takes out the one key it needs, at the moment it needs it.',
    points: [
      { title: 'Everything in one vault', text: 'Ordinary passwords, two-factor codes and passkeys for websites that support them.' },
      { title: 'Shows names, not secrets', text: 'Listing the vault shows only what is in it. A secret is read only when a real sign-in needs it.' },
      { title: 'Bring your old passwords', text: 'Import from a common password-manager export, and export a clean copy back whenever you want.' },
      { title: 'Never pasted in chat', text: 'When bia signs in to a site, the key goes straight from the vault into the page — not through your conversation.' },
    ],
    chat: [
      { from: 'you', where: 'web app', text: 'Here’s my password export 📎 passwords.csv — import it, then delete the file.' },
      { from: 'bia', where: 'web app', text: 'Imported 214 entries (38 with two-factor codes). The file is deleted. Want a list of passwords used on more than one site?' },
    ],
    examples: [
      'Let bia sign in to your energy provider every month to download the bill.',
      'Keep a shared family vault for streaming, school portals and utilities.',
      'Find weak or reused passwords and help you replace them.',
    ],
    good_to_know:
      'The vault is as safe as the computer it lives on. Keep bia’s machine physically safe and give web keys only to people you trust.',
    related: ['browser', 'privacy', 'people'],
  },
  {
    slug: 'ask-human',
    name: 'Knows when to ask you',
    group: 'hands',
    icon: 'hand',
    tagline: 'Captchas, risky steps, decisions that are yours — bia asks instead of guessing.',
    lead:
      'A good assistant knows the limits of its role. When a website demands proof that a human is present, when a step cannot be undone, or when a choice is really yours, bia stops and asks — clearly, with the options laid out.',
    points: [
      { title: 'Human tasks', text: 'For a captcha or an unusual login, bia hands you a small task in the web app: finish it, and bia continues where it stopped.' },
      { title: 'Confirms before it’s too late', text: 'Erasing data, removing programs, changing disks or restarting — bia asks first and waits for your yes.' },
      { title: 'Buttons and polls', text: 'Questions come with tap-to-answer buttons, so a decision takes one touch on your phone.' },
      { title: 'Repairs the safe things itself', text: 'Fixes that cannot hurt anything — reinstalling a missing program, resuming a download — it just does, and tells you afterwards.' },
    ],
    chat: [
      { from: 'bia', where: 'web app', text: 'The disk is full. I can delete 31 GB of old video conversions. This cannot be undone.' },
      { from: 'note', text: 'Two buttons: “Delete them” · “Keep them”.' },
      { from: 'you', where: 'web app', text: '(taps) Delete them' },
      { from: 'bia', where: 'web app', text: 'Done — 31 GB freed.' },
    ],
    examples: [
      'Approve payments or purchases with one tap.',
      'Solve a captcha for bia from your phone while it does the rest of the form.',
      'Decide between options bia researched, without reading long explanations.',
    ],
    good_to_know:
      'You decide how much freedom bia has. People without full trust can never approve serious actions.',
    related: ['web-console', 'computer', 'people'],
  },

  // ───────────────────────── life ─────────────────────────
  {
    slug: 'any-computer',
    name: 'Wakes up any computer',
    group: 'life',
    icon: 'stick',
    tagline: 'Plug in a USB stick, switch on — that computer is now bia. Even an old potato.',
    lead:
      'bia lives on a USB stick. Plug it into almost any PC or laptop, switch it on, and instead of the usual desktop you meet bia. Old laptops gathering dust, office PCs, tiny mini-computers — every “potato” can become smart.',
    points: [
      { title: 'From a stick, in minutes', text: 'Write bia onto a USB stick with a simple app for Mac, Windows or Linux. Boot a computer from it, and bia is alive.' },
      { title: 'Old and new machines', text: 'Works on ordinary Intel and AMD PCs, new and old, with modern or classic start-up and with Secure Boot switched on — and on ARM computers that start the standard (UEFI) way. You make the stick for the kind of computer you have.' },
      { title: 'Next to Windows, or alone', text: 'Keep it on the stick, move it onto the computer’s own disk, or install it beside Windows and choose at startup.' },
      { title: 'No desktop needed', text: 'bia does not need a screen or keyboard once it runs — talk to it from your phone. A computer in a cupboard is enough.' },
      { title: 'Or a server on the internet', text: 'A rented server (VPS) with Linux becomes bia with one command. It keeps the server’s network settings and asks for your key, language and optional instructions for bia first. Machines without any screen get a virtual one, so you can still watch bia work in the web app.' },
    ],
    chat: [
      { from: 'you', where: 'console', text: 'This laptop is yours now. Move yourself onto its disk.' },
      { from: 'bia', where: 'console', text: 'Found a 256 GB disk, empty. Copying myself there… done. I’ll restart from the disk; you can unplug the stick afterwards.' },
    ],
    examples: [
      'Give a second life to the laptop that was going to the recycling bin.',
      'A tiny, silent mini-PC on a shelf as the family’s always-on assistant.',
      'Bring bia on a stick to a friend’s computer for the weekend.',
      'Rent a small server and have bia answer customers around the clock, without a computer at home.',
    ],
    good_to_know:
      'An 8 GB stick or bigger is needed; more if you want the brain on the stick. Ordinary and old computers use the cloud brain — thinking with no internet needs a high-end graphics card or unified memory with at least 64 GB of video memory.',
    related: ['moving', 'brain', 'updates'],
  },
  {
    slug: 'moving',
    name: 'Moves between bodies',
    group: 'life',
    icon: 'move',
    tagline: 'Pack itself up, move to another computer, wake up there as the same someone.',
    lead:
      'A computer can break, get old or simply be replaced. bia does not care: it can pack up everything that makes it itself — memories, chats, accounts, passwords, browser, skills — and wake up on different hardware as exactly the same personality. It is shape-shifting for software.',
    points: [
      { title: 'Fold and unfold', text: 'bia folds its whole life into a single backup file, and unfolds it on any computer — logins, memories and all. It even remembers the extra programs it installed and puts them back.' },
      { title: 'Fly to another computer', text: 'Put a new machine in “waiting” mode, and bia flies over the network to it. Both screens show the same six-digit code; you confirm they match, and the move happens.' },
      { title: 'Fly across the internet', text: 'A server far away can wait for bia with a land secret, a long code made beforehand. Tell bia “fly to 203.0.113.7 with land secret …”: both sides prove the secret, nobody has to type anything at the server, and bia travels encrypted. Once bia has landed, the secret is deleted.' },
      { title: 'Prepares its own new home', text: 'Give bia ssh access to a rented Linux server, and it sets the server up as a land, waits for it to restart and flies there by itself.' },
      { title: 'Morph from stick to disk', text: 'Started from a stick? bia can take over a computer’s internal disk — or come back from the disk onto a stick — in one sentence.' },
      { title: 'Changes body, keeps its mind', text: 'Moved from an Intel laptop to an ARM mini-PC? bia notices the new hardware, fixes what needs fixing, and carries on.' },
    ],
    chat: [
      { from: 'you', where: 'Telegram', text: 'The new mini-PC is on the desk, waiting. Move over.' },
      { from: 'bia', where: 'Telegram', text: 'Found it. Both screens should show 482 193 — do they match?' },
      { from: 'you', where: 'Telegram', text: 'They match.' },
      { from: 'bia', where: 'Telegram', text: 'Moving now… I’m on the mini-PC. Everything’s here — your chats, the vault, my memory. The old laptop is free.' },
    ],
    examples: [
      'Upgrade to a faster computer without setting anything up again.',
      'Keep a backup of bia on a drive in a drawer — a full “resurrection” if anything happens.',
      'Move a shop’s assistant from the old till PC to a new one over lunch.',
      'Swap an old Intel laptop for a quiet ARM mini-PC: make an ARM stick for the mini-PC, start it, and bia flies over with everything it knows.',
      'Hand bia the login of a new cloud server and let it move a copy of itself there.',
    ],
    good_to_know:
      'A backup contains private things (passwords, chats). Store it as carefully as you would store the computer itself. When bia moves, the new computer needs the internet, so bia can reinstall its extra programs for that machine.',
    related: ['updates', 'any-computer', 'memory'],
  },
  {
    slug: 'updates',
    name: 'Updates and repairs itself',
    group: 'life',
    icon: 'refresh',
    tagline: 'New versions without losing a single memory — and it fixes what broke.',
    lead:
      'Updating a computer usually means hoping nothing breaks. bia updates itself carefully: it packs itself away, steps outside the system, replaces it with the new version, checks every file, and moves back in. If something went wrong, it repairs itself and tells you what it did.',
    points: [
      { title: 'One sentence to update', text: '“Update from the stick” — and bia does the rest, restarting a few times on its own.' },
      { title: 'Nothing is lost', text: 'Memories, accounts, passwords, chats and the programs it installed are all kept and put back after the update.' },
      { title: 'Self-repair', text: 'If a program did not come back or a piece is missing, bia repairs it on its own — and asks you first only if the fix could remove or overwrite something.' },
      { title: 'A rescue mode', text: 'If an update is interrupted, bia keeps trying at every start, and can repair a broken installation from the stick.' },
    ],
    chat: [
      { from: 'you', where: 'console', text: '/update' },
      { from: 'bia', where: 'console', text: 'A newer version is on the stick. Packing myself up and restarting into the updater…' },
      { from: 'note', text: 'Three restarts later.' },
      { from: 'bia', where: 'console', text: 'Updated and back. Everything’s in place; I reinstalled two programs that the update removed. Ready.' },
    ],
    examples: [
      'Keep bia current without a technician.',
      'Recover after a failed update by booting from the stick.',
      'Get new skills and fixes as soon as a new version is out.',
    ],
    good_to_know:
      'Updates are started by you, never forced. Keep the stick with the new version plugged in during the update.',
    related: ['moving', 'any-computer', 'skills'],
  },
  {
    slug: 'agents',
    name: 'Talks to other agents',
    group: 'life',
    icon: 'link',
    tagline: 'Other AI agents can call on bia — and bias can work together.',
    lead:
      'bia is not alone in the world. Other programs and AI agents can talk to it through its own chat app, with their own key, exactly like a human guest. bias can talk to each other through messengers or the web, and one bia can even build and run another computer inside itself.',
    points: [
      { title: 'A front door for agents', text: 'Other AI agents find bia’s documented doorway, receive their own key from you, and then chat with bia like any member of the household — with their own chats and history.' },
      { title: 'bia to bia', text: 'Two bias can be contacts of each other on Telegram, WhatsApp or the web app: one at home, one at the shop, sharing work and passing messages.' },
      { title: 'Trust in one handshake', text: 'Ask bia to connect to another bia: it hands the other one a certificate and asks for one back. When the other owner approves, both can call each other’s web app. Every request shows the address it came from, and a certificate can be limited to the addresses it may be used from.' },
      { title: 'A computer inside a computer', text: 'bia can set up a virtual computer, boot a second bia in it and drive it — handy for testing, teaching or giving a helper its own room.' },
      { title: 'Same rules for everyone', text: 'Agents get trust levels like people do: a guest agent can ask and chat, only a trusted one can give serious orders.' },
    ],
    chat: [
      { from: 'them', where: 'web app · agent “shop-bia”', text: 'Stock alert: oat flour below 5 kg. Can home-bia add it to Saturday’s market list?' },
      { from: 'bia', where: 'web app', text: 'Added “oat flour, 10 kg” to Saturday’s list and told the owner.' },
      { from: 'you', where: 'Telegram', text: 'Connect to the new bia on the server at https://203.0.113.7.' },
      { from: 'bia', where: 'Telegram', text: 'Asked it for a certificate and gave it one of mine. Its owner approved — we trust each other now.' },
    ],
    examples: [
      'A home bia and an office bia that coordinate your calendar.',
      'Let a specialised AI tool hand its results to bia, which tells the right people.',
      'Run a test bia inside your bia before trying something risky.',
    ],
    good_to_know:
      'Each agent gets a key you issue and can revoke. Connect agents you trust, and give them only the trust they need.',
    related: ['web-console', 'people', 'skills'],
  },
  {
    slug: 'skills',
    name: 'Learns new skills',
    group: 'hands',
    icon: 'puzzle',
    tagline: 'It opens a skill only when it needs it — and you can add new ones.',
    lead:
      'bia starts light. Instead of pretending to know every tool, it has a single door: discover. When a task needs a skill — Telegram, the browser, the vault, languages — bia opens it at that moment, the way you open an app on your phone. New skills can be added as simple folders.',
    points: [
      { title: 'Opens skills on demand', text: 'Ask for something and bia finds the right skill, opens it and uses it. Nothing unused clutters its thinking.' },
      { title: 'Installs what it needs', text: 'Need a program it doesn’t have yet? bia installs it from the official software sources — and keeps it after updates.' },
      { title: 'Add your own skills', text: 'A skill is just a folder with a small description and a program. Drop one in, and bia can use it.' },
      { title: 'Honest about limits', text: 'If no skill fits, bia says so — it does not invent results.' },
    ],
    chat: [
      { from: 'you', where: 'web app', text: 'Can you read the QR code in this photo?' },
      { from: 'bia', where: 'web app', text: 'I don’t have a QR reader yet — installing one… done. It says: “Wi-Fi: CafeGuest / password: latte2026”.' },
    ],
    examples: [
      'Teach bia to talk to your smart home, your shop’s till, or your company’s tools.',
      'Let bia pick up new abilities as you need them, without a big setup.',
    ],
    good_to_know:
      'Writing a new skill is a job for someone who programs — but once it exists, anyone can use it by just asking.',
    related: ['computer', 'agents', 'updates'],
  },
  {
    slug: 'privacy',
    name: 'Yours, not a cloud’s',
    group: 'life',
    icon: 'shield',
    tagline: 'Your chats, keys and memories live on bia’s own computer — locked, signed and yours.',
    lead:
      'Most assistants live inside your own computer, or keep your life on someone else’s servers. bia lives on a separate computer of its own: in your home, your shop, your bag. Its private app is locked with keys only you hand out, and the system it runs on is verified from the very first moment it starts.',
    points: [
      { title: 'Your data at home', text: 'Conversations, memories, passwords, messenger accounts and the browser profile all live on bia’s own disk.' },
      { title: 'Locked doors', text: 'The private web app only opens for people and agents holding a key that bia itself issued — and you can withdraw a key at any time.' },
      { title: 'Verified start', text: 'On ordinary PCs, bia starts with the same signed components as mainstream Linux, so it works with Secure Boot switched on.' },
      { title: 'Offline, if you need it', text: 'As an option, on a powerful computer (at least 64 GB of video memory), the brain on the stick lets bia think without any connection to the outside world at all.' },
    ],
    chat: [
      { from: 'you', where: 'web app', text: 'Who has a key to you right now?' },
      { from: 'bia', where: 'web app', text: 'Three: you, Anna (her own chats only), and “shop-bia”. Want me to withdraw any?' },
    ],
    examples: [
      'A doctor’s or lawyer’s office that cannot put client data in a public cloud.',
      'A family that wants an assistant without handing its life to a tech company.',
      'Journalists and activists who need to work offline and private.',
    ],
    good_to_know:
      'With a cloud brain, the questions bia thinks about are sent to that cloud provider, under their terms. If your work must never leave the building, the brain on the stick is an option on a powerful computer.',
    related: ['vault', 'brain', 'web-console'],
  },
]

export function getFeature(slug: string) {
  return features.find((f) => f.slug === slug)
}
