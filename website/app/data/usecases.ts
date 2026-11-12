// Who bia is for: three kinds of small circles of people, each with what it changes and one
// ordinary day told through who bia talked to, where, and what it did on its own.

export type DayStep = {
  time: string
  /** who bia is dealing with */
  who: string
  /** where the conversation happens */
  where: string
  what: string
  /** bia started this itself, nobody asked at that moment */
  own?: boolean
}

export type UseCaseGroup = {
  slug: string
  title: string
  icon: string
  /** one line for cards */
  intro: string
  /** the idea, in a paragraph */
  lead: string
  benefits: { title: string; text: string }[]
  day: { title: string; setting: string; steps: DayStep[] }
  /** feature slugs that make it work */
  uses: string[]
}

export const useCaseGroups: UseCaseGroup[] = [
  {
    slug: 'family',
    title: 'For a family',
    icon: 'home',
    intro: 'One more member of the household, known to everyone — not a subscription and a private assistant for each person.',
    lead:
      'A family is not five separate users. It is one web of people who depend on each other, and most of what goes wrong goes wrong between them: who picks up whom, did grandma take her pills, who was going to call the plumber. bia joins that web as one someone the whole family knows. It has its own number and accounts, sits in the family chat, talks to each person in their own chat and language, and remembers what it promised to whom.',
    benefits: [
      {
        title: 'One bia for everybody',
        text: 'One computer, one brain bill, one personality. Grandparents, parents and kids all write to the same someone, each from the messenger they already use. Nobody installs or pays for anything of their own.',
      },
      {
        title: 'It knows how you belong together',
        text: 'bia keeps its own picture of the family: who is whose mother, who is away this week, who answers only by voice. So “tell Dad I’ll be late” works, and a message from grandpa reaches the right person without anyone being asked twice.',
      },
      {
        title: 'It carries things between people',
        text: 'Personal assistants each serve one person and never talk to each other. bia passes the message on, checks that it arrived, and comes back with the answer — the part of family life that usually lands on one tired parent.',
      },
      {
        title: 'Private where it should be',
        text: 'Each person has their own chat with bia, and it does not retell one in the other. What it shares, it shares the way a sensible relative would: the pills were taken — yes; what you two talked about — no.',
      },
      {
        title: 'It starts things itself',
        text: 'A good-morning note, the pill call at nine, “the school trip form is due tomorrow”, a nudge when nobody has answered grandma since yesterday. Nobody has to open an app and ask.',
      },
      {
        title: 'The family’s data stays in the family',
        text: 'bia lives on its own computer: an old laptop in the cupboard, or a small server of its own. Chats, memories and passwords are kept there, not in a company’s account for each of you.',
      },
    ],
    day: {
      title: 'A Thursday with the Novaks',
      setting:
        'Marta and Jan, their kids Ela (15) and Tomek (9), and Marta’s mother Halina, who lives across town. Their bia is called Bruno. It runs on Jan’s old laptop and has its own WhatsApp number and Telegram account.',
      steps: [
        { time: '06:45', who: 'Family group', where: 'WhatsApp', own: true, what: 'Bruno posts the day: rain until noon, Tomek’s swimming bag, Jan’s late meeting, and “Ela, your maths test is today — good luck”.' },
        { time: '09:00', who: 'Halina', where: 'WhatsApp call', own: true, what: 'Bruno calls grandma, as every morning: “Good morning, Halina. Did you take the white pill?” She did. They chat a minute about her knee. Bruno writes to Marta: “Mum took her pills. Her knee is worse than yesterday — maybe call her today.”' },
        { time: '11:20', who: 'Ela', where: 'Telegram', what: 'Ela, between classes: “Can I stay at Kasia’s after school?” That is not Bruno’s to decide. It asks Marta with two buttons, gets a yes until seven, tells Ela, and puts “Ela home by 19:00” in the family calendar.' },
        { time: '13:05', who: 'The plumber', where: 'WhatsApp', what: 'Jan asked yesterday to get the tap fixed. Bruno has been writing with the plumber as itself — “I am Bruno, the Novak family’s assistant” — and settles Saturday at ten. It tells Jan and adds it to the calendar.' },
        { time: '15:30', who: 'Tomek', where: 'Voice message', what: 'Tomek sends a voice message: he forgot which bus goes to the pool. Bruno answers by voice, with the number and the stop, and tells the group he is on his way.' },
        { time: '17:40', who: 'Halina → Marta', where: 'WhatsApp', what: 'Halina sends a photo of a letter from the clinic she cannot read. Bruno reads it, explains it to her in plain words, and forwards it to Marta with one line: an appointment on the 21st, someone should drive her.' },
        { time: '19:10', who: 'Ela', where: 'Telegram', own: true, what: 'Ela is not home. Bruno writes to her first, not to her parents: “It’s ten past seven — on your way?” She is, five minutes out. Nothing needs to reach Marta.' },
        { time: '21:30', who: 'Marta and Jan', where: 'Their own chats', own: true, what: 'A short evening note for each: what is open for tomorrow, who promised what. Then Bruno writes the day into its diary.' },
      ],
    },
    uses: ['messengers', 'voice', 'people', 'calendar', 'ask-human', 'memory', 'privacy'],
  },
  {
    slug: 'business',
    title: 'For a small business',
    icon: 'shop',
    intro: 'A real, always-on colleague: it talks to customers, suppliers and the team, and works with the company’s own tools.',
    lead:
      'A small business runs on conversations: customers who write at night, a supplier who is late, an employee who swaps a shift, an owner who wants the numbers on Friday. bia is not a chat widget on the website. It is a member of staff with its own phone number, accounts and desk — its own computer — who knows the customers by name, knows the team, and deals with the business itself: the booking system, the shared calendar, the inbox, the spreadsheets.',
    benefits: [
      {
        title: 'On shift all day, every day',
        text: 'Customers get an answer in seconds on WhatsApp, Instagram, Telegram, Messenger or the phone, in their own language — at midnight and on Sunday too.',
      },
      {
        title: 'It talks to the company, not only for it',
        text: 'bia asks the cook what is off the menu, reminds the driver, chases the supplier, and tells the owner what needs a decision. Inside and outside, it is the same colleague.',
      },
      {
        title: 'It works in your systems',
        text: 'With its own logins, bia uses the booking site, the till’s reports, email, the shared calendar and the accounting portal the way an employee would, in a real browser on its own computer.',
      },
      {
        title: 'It remembers every customer',
        text: 'Who ordered what, what was promised and when, who prefers a call. The memory belongs to the business and stays on its machine when staff change.',
      },
      {
        title: 'It knows what is not its call',
        text: 'Refunds, discounts, an angry customer: bia asks the right person with a clear question and two buttons, and tells the customer honestly that it is checking.',
      },
      {
        title: 'One colleague, not a seat per employee',
        text: 'One bia serves the whole shop. It costs a small computer and what its thinking uses — no per-user plans, and the customer list never leaves your premises.',
      },
    ],
    day: {
      title: 'A Friday at a bike repair shop',
      setting:
        'Three people: Ines, the owner; Karim, the mechanic; and Sofia, who works the counter on weekends. Their bia is called Rita. It has the shop’s second phone number, an Instagram account of its own, and a login to the booking calendar and the parts supplier’s site.',
      steps: [
        { time: '00:40', who: 'A customer', where: 'Instagram', what: 'A message under last week’s post: “Do you fix e-bike brakes? How much?” Rita answers with the price range, offers Tuesday or Wednesday, and books Wednesday at nine in the shop calendar.' },
        { time: '08:15', who: 'Karim', where: 'Telegram', own: true, what: 'Before he opens: today’s six jobs in order, which parts are in the drawer, and one warning — the cassette for the green Trek did not arrive.' },
        { time: '08:20', who: 'The supplier', where: 'Email and their website', own: true, what: 'Rita signs in to the supplier’s site, sees the parcel is stuck, writes to their sales contact as “Rita from the shop”, and gets a new delivery day: Monday.' },
        { time: '08:40', who: 'The Trek’s owner', where: 'WhatsApp', own: true, what: 'Rita tells the customer before he asks: the part is late, the bike will be ready Tuesday, sorry. He answers with a thumbs-up instead of a bad review.' },
        { time: '11:05', who: 'A tourist', where: 'Phone call', what: 'A call in German: a flat tyre, can he come now? Rita answers in German, checks with Karim in the chat — yes, twenty minutes — and gives directions.' },
        { time: '14:30', who: 'Ines', where: 'Telegram', what: 'A customer wants money back for a repair from March. Rita does not decide that. It sends Ines the history of the job, what was promised, and two buttons: refund half, or invite him in.' },
        { time: '16:00', who: 'Sofia', where: 'WhatsApp', what: 'Sofia cannot work Saturday morning. Rita asks Karim if he can open, gets a yes, changes the shift in the calendar and confirms to both.' },
        { time: '18:30', who: 'Ines', where: 'Telegram', own: true, what: 'The Friday note: jobs done, money in, three bikes waiting for parts, two customers who have not answered about pick-up — Rita will ask them again on Monday.' },
      ],
    },
    uses: ['messengers', 'voice', 'languages', 'browser', 'vault', 'people', 'ask-human', 'calendar'],
  },
  {
    slug: 'communities',
    title: 'For a small community',
    icon: 'people',
    intro: 'A club, a house, a choir, a volunteer group: bia organises things, plans meetings and keeps everyone in the loop.',
    lead:
      'Small communities live on the unpaid work of one or two people who remember everything and chase everyone. bia takes that work. It is a member of the group with its own account: it finds a date that suits most people, reminds those who forgot, welcomes the new ones, knows who has the key and who paid, and writes to people one by one when the group chat would be too loud.',
    benefits: [
      {
        title: 'It gets meetings to happen',
        text: 'bia asks who can come when, counts the answers, nudges the silent ones in private, books the room and sends the calendar invitation. The date is settled without forty messages.',
      },
      {
        title: 'It knows everybody',
        text: 'Names, roles, languages, who joined when, who knows whom. A newcomer gets a proper welcome and an introduction to the right person, not a link to a pinned message.',
      },
      {
        title: 'It keeps the list',
        text: 'Fees, shifts, who brings what, who borrowed the projector. bia keeps the record, reminds politely and in private, and shows the organiser the whole picture.',
      },
      {
        title: 'Quiet in the group, attentive in private',
        text: 'It speaks in the group when there is something for everyone, and writes personally when a matter concerns one person. It answers the usual questions so the organisers do not have to.',
      },
      {
        title: 'Everyone in their own language',
        text: 'Members write and send voice messages in the language they are comfortable in. bia answers in it, and passes things on to the others in theirs.',
      },
      {
        title: 'The community owns it',
        text: 'bia lives on a computer the group controls. The member list and the chats are not handed to a platform, and when the organiser changes, the memory stays.',
      },
    ],
    day: {
      title: 'A Monday with a neighbourhood choir',
      setting:
        'Twenty-eight singers, one conductor, a rehearsal room in a school that is not always free. Their bia is called Mila. She is in the choir’s Telegram group, has a WhatsApp number for the members who are not on Telegram, and holds the choir’s calendar.',
      steps: [
        { time: '08:00', who: 'The school’s secretary', where: 'Email', own: true, what: 'The room is taken on Thursday. Mila knew since Friday and has been asking for another slot; the secretary now offers Wednesday or the hall on Thursday.' },
        { time: '08:30', who: 'The whole choir', where: 'Telegram poll, WhatsApp', own: true, what: 'Mila asks: Wednesday in the room, or Thursday in the hall? A poll in the group, and the same question in person to the six members on WhatsApp.' },
        { time: '12:00', who: 'Nine people who did not answer', where: 'Private chats', own: true, what: 'Mila writes to each of them, in their language — two in Ukrainian, one in Spanish. By two o’clock, twenty-four have answered: Thursday, in the hall.' },
        { time: '14:10', who: 'The conductor', where: 'Telegram', what: 'Mila tells Ana the result and that both sopranos with the solo can come. Ana says yes. Mila confirms the hall with the school and sends everyone a calendar invitation.' },
        { time: '15:30', who: 'A newcomer', where: 'WhatsApp', what: 'Someone found the choir’s poster and writes: can anyone join? Mila answers, asks which voice she sings, sends the address and Thursday’s time, and introduces her to the alto section’s leader.' },
        { time: '17:00', who: 'Four members', where: 'Private chats', own: true, what: 'The yearly fee was due last week. Mila reminds the four who have not paid, one by one and kindly. Two pay tonight; she notes it and tells the treasurer.' },
        { time: '19:45', who: 'Marek', where: 'Voice message', what: 'Marek has the key to the hall cupboard and will be late on Thursday. Mila asks who lives near him, finds Olga, and arranges the hand-over between the two.' },
        { time: '21:00', who: 'The conductor', where: 'Telegram', own: true, what: 'A short summary for Ana: Thursday is set, twenty-four coming, one new alto, fees at 26 of 28, the key is sorted.' },
      ],
    },
    uses: ['messengers', 'people', 'calendar', 'languages', 'voice', 'memory'],
  },
]

/** Things people ask a bia for that fit no category: it is social and has a computer, like a person. */
export const ownUses: string[] = [
  'Be the contact person for a wedding: guests, suppliers, the seating plan, the day itself.',
  'Look after an elderly neighbour with a daily call, and tell her children when something sounds wrong.',
  'Run a flat-share: the rota, the shared bills, the landlord, the person who never does the bins.',
  'Keep a freelancer’s clients answered while she works, and chase the unpaid invoices politely.',
  'Welcome guests of a holiday flat in their language and send the cleaner the times.',
  'Organise the school class’s parents: the present for the teacher, who drives to the match.',
  'Sit in a research group’s chat, keep the reading list and remind people of the deadline.',
  'Set up and look after other bias: a second one for the shop, a third for the grandparents.',
]
