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

import type { T } from './features'

export const buildUseCaseGroups = (t: T): UseCaseGroup[] => [
  {
    slug: 'family',
    title: t('usecases:for_a_family'),
    icon: 'home',
    intro: t('usecases:one_more_member_of_the_household'),
    lead:
      t('usecases:a_family_is_not_five_separate'),
    benefits: [
      {
        title: t('usecases:one_bia_for_everybody'),
        text: t('usecases:one_computer_one_brain_bill_one'),
      },
      {
        title: t('usecases:it_knows_how_you_belong_together'),
        text: t('usecases:bia_keeps_its_own_picture_of'),
      },
      {
        title: t('usecases:it_carries_things_between_people'),
        text: t('usecases:personal_assistants_each_serve_one'),
      },
      {
        title: t('usecases:private_where_it_should_be'),
        text: t('usecases:each_person_has_their_own_chat'),
      },
      {
        title: t('usecases:it_starts_things_itself'),
        text: t('usecases:a_good_morning_note_the_pill'),
      },
      {
        title: t('usecases:the_familys_data_stays_in_the'),
        text: t('usecases:bia_lives_on_its_own_computer'),
      },
    ],
    day: {
      title: t('usecases:a_thursday_with_the_novaks'),
      setting:
        t('usecases:marta_and_jan_their_kids_ela'),
      steps: [
        { time: '06:45', who: t('usecases:family_group'), where: t('usecases:whatsapp'), own: true, what: t('usecases:bruno_posts_the_day_rain_until') },
        { time: '09:00', who: t('usecases:halina'), where: t('usecases:whatsapp_call'), own: true, what: t('usecases:bruno_calls_grandma_as_every_morning') },
        { time: '11:20', who: t('usecases:ela'), where: t('usecases:telegram'), what: t('usecases:ela_between_classes_can_i_stay') },
        { time: '13:05', who: t('usecases:the_plumber'), where: t('usecases:whatsapp'), what: t('usecases:jan_asked_yesterday_to_get_the') },
        { time: '15:30', who: t('usecases:tomek'), where: t('usecases:voice_message'), what: t('usecases:tomek_sends_a_voice_message_he') },
        { time: '17:40', who: t('usecases:halina_marta'), where: t('usecases:whatsapp'), what: t('usecases:halina_sends_a_photo_of_a') },
        { time: '19:10', who: t('usecases:ela'), where: t('usecases:telegram'), own: true, what: t('usecases:ela_is_not_home_bruno_writes') },
        { time: '21:30', who: t('usecases:marta_and_jan'), where: t('usecases:their_own_chats'), own: true, what: t('usecases:a_short_evening_note_for_each') },
      ],
    },
    uses: ['messengers', 'voice', 'people', 'calendar', 'ask-human', 'memory', 'privacy'],
  },
  {
    slug: 'business',
    title: t('usecases:for_a_small_business'),
    icon: 'shop',
    intro: t('usecases:a_real_always_on_colleague_it'),
    lead:
      t('usecases:a_small_business_runs_on_conversations'),
    benefits: [
      {
        title: t('usecases:on_shift_all_day_every_day'),
        text: t('usecases:customers_get_an_answer_in_seconds'),
      },
      {
        title: t('usecases:it_talks_to_the_company_not'),
        text: t('usecases:bia_asks_the_cook_what_is'),
      },
      {
        title: t('usecases:it_works_in_your_systems'),
        text: t('usecases:with_its_own_logins_bia_uses'),
      },
      {
        title: t('usecases:it_remembers_every_customer'),
        text: t('usecases:who_ordered_what_what_was_promised'),
      },
      {
        title: t('usecases:it_knows_what_is_not_its'),
        text: t('usecases:refunds_discounts_an_angry_customer'),
      },
      {
        title: t('usecases:one_colleague_not_a_seat_per'),
        text: t('usecases:one_bia_serves_the_whole_shop'),
      },
    ],
    day: {
      title: t('usecases:a_friday_at_a_bike_repair'),
      setting:
        t('usecases:three_people_ines_the_owner_karim'),
      steps: [
        { time: '00:40', who: t('usecases:a_customer'), where: t('usecases:instagram'), what: t('usecases:a_message_under_last_weeks_post') },
        { time: '08:15', who: t('usecases:karim'), where: t('usecases:telegram'), own: true, what: t('usecases:before_he_opens_todays_six_jobs') },
        { time: '08:20', who: t('usecases:the_supplier'), where: t('usecases:email_and_their_website'), own: true, what: t('usecases:rita_signs_in_to_the_suppliers') },
        { time: '08:40', who: t('usecases:the_treks_owner'), where: t('usecases:whatsapp'), own: true, what: t('usecases:rita_tells_the_customer_before_he') },
        { time: '11:05', who: t('usecases:a_tourist'), where: t('usecases:phone_call'), what: t('usecases:a_call_in_german_a_flat') },
        { time: '14:30', who: t('usecases:ines'), where: t('usecases:telegram'), what: t('usecases:a_customer_wants_money_back_for') },
        { time: '16:00', who: t('usecases:sofia'), where: t('usecases:whatsapp'), what: t('usecases:sofia_cannot_work_saturday_morning') },
        { time: '18:30', who: t('usecases:ines'), where: t('usecases:telegram'), own: true, what: t('usecases:the_friday_note_jobs_done_money') },
      ],
    },
    uses: ['messengers', 'voice', 'languages', 'browser', 'vault', 'people', 'ask-human', 'calendar'],
  },
  {
    slug: 'communities',
    title: t('usecases:for_a_small_community'),
    icon: 'people',
    intro: t('usecases:a_club_a_house_a_choir'),
    lead:
      t('usecases:small_communities_live_on_the_unpaid'),
    benefits: [
      {
        title: t('usecases:it_gets_meetings_to_happen'),
        text: t('usecases:bia_asks_who_can_come_when'),
      },
      {
        title: t('usecases:it_knows_everybody'),
        text: t('usecases:names_roles_languages_who_joined_when'),
      },
      {
        title: t('usecases:it_keeps_the_list'),
        text: t('usecases:fees_shifts_who_brings_what_who'),
      },
      {
        title: t('usecases:quiet_in_the_group_attentive_in'),
        text: t('usecases:it_speaks_in_the_group_when'),
      },
      {
        title: t('usecases:everyone_in_their_own_language'),
        text: t('usecases:members_write_and_send_voice_messages'),
      },
      {
        title: t('usecases:the_community_owns_it'),
        text: t('usecases:bia_lives_on_a_computer_the'),
      },
    ],
    day: {
      title: t('usecases:a_monday_with_a_neighbourhood_choir'),
      setting:
        t('usecases:twenty_eight_singers_one_conductor_a'),
      steps: [
        { time: '08:00', who: t('usecases:the_schools_secretary'), where: t('usecases:email'), own: true, what: t('usecases:the_room_is_taken_on_thursday') },
        { time: '08:30', who: t('usecases:the_whole_choir'), where: t('usecases:telegram_poll_whatsapp'), own: true, what: t('usecases:mila_asks_wednesday_in_the_room') },
        { time: '12:00', who: t('usecases:nine_people_who_did_not_answer'), where: t('usecases:private_chats'), own: true, what: t('usecases:mila_writes_to_each_of_them') },
        { time: '14:10', who: t('usecases:the_conductor'), where: t('usecases:telegram'), what: t('usecases:mila_tells_ana_the_result_and') },
        { time: '15:30', who: t('usecases:a_newcomer'), where: t('usecases:whatsapp'), what: t('usecases:someone_found_the_choirs_poster_and') },
        { time: '17:00', who: t('usecases:four_members'), where: t('usecases:private_chats'), own: true, what: t('usecases:the_yearly_fee_was_due_last') },
        { time: '19:45', who: t('usecases:marek'), where: t('usecases:voice_message'), what: t('usecases:marek_has_the_key_to_the') },
        { time: '21:00', who: t('usecases:the_conductor'), where: t('usecases:telegram'), own: true, what: t('usecases:a_short_summary_for_ana_thursday') },
      ],
    },
    uses: ['messengers', 'people', 'calendar', 'languages', 'voice', 'memory'],
  },
]

/** Things people ask a bia for that fit no category: it is social and has a computer, like a person. */
export const buildOwnUses = (t: T): string[] => [
  t('usecases:be_the_contact_person_for_a'),
  t('usecases:look_after_an_elderly_neighbour_with'),
  t('usecases:run_a_flat_share_the_rota'),
  t('usecases:keep_a_freelancers_clients_answered'),
  t('usecases:welcome_guests_of_a_holiday_flat'),
  t('usecases:organise_the_school_classs_parents_the'),
  t('usecases:sit_in_a_research_groups_chat'),
  t('usecases:set_up_and_look_after_other'),
]
