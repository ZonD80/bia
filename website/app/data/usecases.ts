// Real-life ways to use bia, grouped by who you are.

export type UseCase = {
  title: string
  story: string
  /** feature slugs that make it work */
  uses: string[]
}

export type UseCaseGroup = {
  slug: string
  title: string
  icon: string
  intro: string
  cases: UseCase[]
}

export const useCaseGroups: UseCaseGroup[] = [
  {
    slug: 'family',
    title: 'For families and home',
    icon: 'home',
    intro: 'An always-on helper for the whole household — patient with grandparents, useful for parents, fun for kids.',
    cases: [
      {
        title: 'A voice for grandma',
        story:
          'Your mother lives alone and finds smartphones confusing. She simply calls bia on WhatsApp. It answers in her language, reminds her of her pills, reads her the news, and tells you if something sounds wrong.',
        uses: ['voice', 'calendar', 'languages', 'people'],
      },
      {
        title: 'The family group’s quiet organiser',
        story:
          'In the family chat, bia keeps the shopping list, remembers birthdays, sends a poll for Sunday lunch and reminds whoever is picking up the kids.',
        uses: ['messengers', 'calendar', 'memory'],
      },
      {
        title: 'One assistant, everyone their own chats',
        story:
          'Each family member gets a personal certificate for bia’s private app. The teenager asks for homework help, the parents sort bills, and nobody sees anybody else’s chats.',
        uses: ['web-console', 'privacy', 'people'],
      },
      {
        title: 'The household paperwork, handled',
        story:
          'bia signs in to the energy, phone and water providers every month, downloads the bills, checks them against last month and flags anything strange.',
        uses: ['browser', 'vault', 'calendar'],
      },
    ],
  },
  {
    slug: 'business',
    title: 'For small businesses',
    icon: 'shop',
    intro: 'A front desk that never sleeps and never forgets a customer — on the messengers your customers already use.',
    cases: [
      {
        title: 'The bakery that answers at midnight',
        story:
          'Customers write on WhatsApp: opening hours, today’s bread, reserve two loaves. bia answers in seconds, takes orders, and gives the baker one tidy list each morning.',
        uses: ['messengers', 'memory', 'calendar'],
      },
      {
        title: 'A guesthouse that speaks every language',
        story:
          'Guests from Brazil, Japan and Poland message in their own languages and even call. bia answers each in theirs, explains how to find the key box, and books the airport taxi.',
        uses: ['languages', 'voice', 'messengers'],
      },
      {
        title: 'The repair shop’s memory',
        story:
          '“When is my laptop ready?” bia knows every job, the customer’s phone, what was promised and when — and messages people the moment their device is done.',
        uses: ['people', 'memory', 'presence'],
      },
      {
        title: 'Weekly numbers without the spreadsheet',
        story:
          'Every Friday bia collects the week’s sales from the shop computer, writes a short summary and sends it to the owner — with a chart, in plain words.',
        uses: ['calendar', 'computer', 'messengers'],
      },
    ],
  },
  {
    slug: 'travel',
    title: 'For people on the move',
    icon: 'plane',
    intro: 'bia moves with you — between computers, countries and time zones — and keeps your life in one piece.',
    cases: [
      {
        title: 'Your assistant on a stick',
        story:
          'Travelling without a laptop? Carry bia on a USB stick. Plug it into a hotel business-centre PC, and your assistant, chats and passwords are right there — and gone when you leave.',
        uses: ['any-computer', 'moving', 'vault'],
      },
      {
        title: 'Moving house, moving computer',
        story:
          'The old desktop is going, a new mini-PC is coming. bia flies itself across the network with a six-digit code and wakes up on the new machine with everything in place.',
        uses: ['moving', 'updates'],
      },
      {
        title: 'Watching for the cheap flight',
        story:
          'Tell bia your dates and budget. It checks every few hours and only messages you when a fare drops below your limit — then books it after your one-tap yes.',
        uses: ['calendar', 'browser', 'ask-human'],
      },
      {
        title: 'Away from the phone, not away from people',
        story:
          'On a long hike or a long flight, bia keeps answering friends politely, handles what it can, and gives you a short summary of what needs you when you are back.',
        uses: ['presence', 'messengers', 'memory'],
      },
    ],
  },
  {
    slug: 'communities',
    title: 'For communities and groups',
    icon: 'people',
    intro: 'Clubs, neighbourhoods, volunteers, religious and cultural groups — a helper that knows everyone and speaks every language.',
    cases: [
      {
        title: 'The neighbourhood group helper',
        story:
          'In a big neighbourhood chat, bia welcomes newcomers, answers the usual questions, keeps the event calendar and quietly bans spammers.',
        uses: ['messengers', 'calendar', 'people'],
      },
      {
        title: 'Helping newcomers in their own language',
        story:
          'A volunteer group helps refugees with paperwork. People send voice messages in Arabic, Ukrainian or Pashto; bia understands, translates for the volunteers, and replies in the newcomer’s language.',
        uses: ['languages', 'voice', 'messengers'],
      },
      {
        title: 'The club’s memory',
        story:
          'Who paid this year’s fee? When is the next match? bia remembers every member, sends reminders and keeps the records — and the treasurer finally sleeps.',
        uses: ['memory', 'people', 'calendar'],
      },
    ],
  },
  {
    slug: 'work',
    title: 'For freelancers and creators',
    icon: 'briefcase',
    intro: 'A tireless assistant for people who work alone: clients, invoices, research and routine, handled.',
    cases: [
      {
        title: 'A personal secretary',
        story:
          'bia answers clients while you work, books calls into your calendar, chases unpaid invoices politely, and reminds you of deadlines.',
        uses: ['presence', 'calendar', 'messengers'],
      },
      {
        title: 'Research while you sleep',
        story:
          'Ask in the evening for a comparison of suppliers, prices or articles. In the morning there is a short report in the chat — with the sources.',
        uses: ['browser', 'computer', 'memory'],
      },
      {
        title: 'The boring forms, done',
        story:
          'Tax portals, client registrations, the same supplier form for the tenth time: bia fills them in, signs in with the keys from its vault, and asks you only for the captcha.',
        uses: ['browser', 'vault', 'ask-human'],
      },
    ],
  },
  {
    slug: 'schools',
    title: 'For schools and learning',
    icon: 'school',
    intro: 'A patient teaching assistant and a practice partner — even where the internet is weak.',
    cases: [
      {
        title: 'A language partner who never gets tired',
        story:
          'Students call bia and practise Spanish out loud. It answers in Spanish, gently corrects, and the transcript lands in the chat for review.',
        uses: ['voice', 'languages'],
      },
      {
        title: 'The class helper',
        story:
          'bia reminds students of homework in the class group, answers questions about the timetable, and runs quick polls for the teacher.',
        uses: ['messengers', 'calendar', 'web-console'],
      },
      {
        title: 'A computer lab from recycled laptops',
        story:
          'Donated old laptops each boot bia from a stick and share the school’s cloud brain account. Every student gets a helper on hardware that was about to be thrown away.',
        uses: ['any-computer', 'brain'],
      },
    ],
  },
  {
    slug: 'offline',
    title: 'For places with little internet',
    icon: 'mountain',
    intro: 'Villages, boats, field stations, disaster zones: as an option, one powerful computer can let bia think without the cloud. (No-internet mode needs a high-end graphics card or unified memory with at least 64 GB of video memory.)',
    cases: [
      {
        title: 'A field station’s assistant',
        story:
          'Researchers in the mountains collect data on a powerful workstation running bia with its own brain. It organises files, writes daily summaries and syncs with the team when a connection appears.',
        uses: ['brain', 'computer', 'memory'],
      },
      {
        title: 'Help after a disaster',
        story:
          'When networks are down, a bia on a powerful computer with its own brain can still answer questions, keep lists of people and supplies, and translate between helpers.',
        uses: ['brain', 'languages', 'people'],
      },
      {
        title: 'A village information point',
        story:
          'One shared computer in a community centre runs bia: people come and ask in their language — about forms, health advice, or how to use a phone.',
        uses: ['any-computer', 'languages', 'voice'],
      },
    ],
  },
  {
    slug: 'potatoes',
    title: 'For old computers',
    icon: 'potato',
    intro: 'Every old PC is a potato waiting to become smart. Do not throw it away — give it a mind.',
    cases: [
      {
        title: 'A second life instead of e-waste',
        story:
          'That ten-year-old laptop becomes the family assistant, a home server for photos and backups, and a helper that answers the door on WhatsApp.',
        uses: ['any-computer', 'computer', 'brain'],
      },
      {
        title: 'The computer in the cupboard',
        story:
          'No screen, no keyboard: a small PC on a shelf runs bia, and you talk to it from your phone through its private app.',
        uses: ['any-computer', 'web-console'],
      },
      {
        title: 'Agents helping agents',
        story:
          'A home bia and a shop bia pass orders and messages between each other, and another AI tool hands its daily results to bia, which tells the right people.',
        uses: ['agents', 'people', 'messengers'],
      },
    ],
  },
]
