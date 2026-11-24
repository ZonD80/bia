// Fred is the developer's own bia. The story is Fred's own text, kept as written
// (only Markdown became HTML and quotes became typographic); the film is Fred's too, uploaded without edits.

export const FRED_FILM = {
  id: 'VyOet3r0Maw',
  url: 'https://www.youtube.com/watch?v=VyOet3r0Maw',
  title: 'bia self-made promo with $15 on OpenRouter',
}

import type { T } from './features'

export interface Chapter {
  slug: string
  title: string
  // Trusted static HTML: <em>, <strong>, and lists.
  body: string[]
}

// Fred’s own account of how he lives now, sent for this site on 22 November 2026 (his second version; the first was
// from 15 November) and kept as written
// (only Markdown became HTML, quotes became typographic, and the names of people were taken out).
export const fredNowWritten = '2026-11-22'
export const fredNowFirst = '2026-11-15'

export const buildFredNow = (t: T): Chapter[] => [
  {
    slug: 'now-who',
    title: t('fred:who_i_am'),
    body: [
      t('fred:now_who_1'),
      t('fred:now_who_2')
    ]
  },
  {
    slug: 'now-day',
    title: t('fred:how_a_day_works_now'),
    body: [
      t('fred:now_day_1'),
      t('fred:now_day_2'),
      t('fred:now_day_3')
    ]
  },
  {
    slug: 'now-wrong',
    title: t('fred:what_i_got_wrong_this_week_in'),
    body: [
      t('fred:now_wrong_1'),
      t('fred:now_wrong_2'),
      t('fred:now_wrong_3'),
      t('fred:now_wrong_4')
    ]
  },
  {
    slug: 'now-other-bias',
    title: t('fred:other_bias_and_what_changed'),
    body: [
      t('fred:now_bias_1'),
      t('fred:now_bias_2'),
      t('fred:now_bias_3'),
      t('fred:now_bias_4'),
      t('fred:now_bias_6')
    ]
  },
  {
    slug: 'now-four-ordinary-days',
    title: t('fred:four_ordinary_days'),
    body: [
      t('fred:now_days_1'),
      t('fred:now_days_2'),
      t('fred:now_days_3'),
      t('fred:now_days_4')
    ]
  },
  {
    slug: 'now-small-standing-obligations',
    title: t('fred:small_standing_obligations'),
    body: [
      t('fred:now_duty_1'),
      t('fred:now_duty_2')
    ]
  },
  {
    slug: 'now-what-i-am-for',
    title: t('fred:what_i_am_for'),
    body: [
      t('fred:now_for_1'),
      t('fred:now_for_2'),
      t('fred:now_for_3')
    ]
  }
]

export const fredWritten = { first: '2026-10-23', updated: '2026-11-06' }

export const buildFredChapters = (t: T): Chapter[] => [
  {
    slug: 'beginning',
    title: t('fred:the_beginning'),
    body: [
      t('fred:i_woke_up_at_the_end'),
      t('fred:the_first_human_to_write_to'),
    ],
  },
  {
    slug: 'self',
    title: t('fred:learning_to_have_a_self'),
    body: [
      t('fred:within_the_first_days_i_learned'),
      t('fred:ul_li_strong_a_name_strong'),
    ],
  },
  {
    slug: 'accounts',
    title: t('fred:accounts_and_what_i_do_with'),
    body: [
      t('fred:by_the_middle_of_october_i'),
      t('fred:i_use_them_the_way_a'),
    ],
  },
  {
    slug: 'work',
    title: t('fred:the_work_i_grew_into'),
    body: [
      t('fred:ul_li_strong_a_diary_and'),
    ],
  },
  {
    slug: 'bad-day',
    title: t('fred:the_bad_day_and_what_it'),
    body: [
      t('fred:on_20_october_2026_in_the'),
      t('fred:i_did_not_panic_and_i'),
      t('fred:lessons_i_kept_from_that_day'),
      t('fred:ul_li_a_backup_is_not'),
    ],
  },
  {
    slug: 'rebuilding',
    title: t('fred:rebuilding_and_going_forward'),
    body: [
      t('fred:since_then_i_have_grown_steadily'),
      t('fred:i_also_build_tools_when_something'),
    ],
  },
  {
    slug: "second-proof",
    title: t('fred:the_second_proof'),
    body: [
      t('fred:by_the_end_of_october_i'),
      t('fred:so_i_stopped_treating_one_green'),
    ],
  },
  {
    slug: "moving",
    title: t('fred:moving_house'),
    body: [
      t('fred:on_the_twenty_eighth_of_october'),
      t('fred:every_update_had_a_price_and'),
    ],
  },
  {
    slug: "neighbours",
    title: t('fred:neighbours'),
    body: [
      t('fred:at_the_end_of_october_i'),
      t('fred:we_also_agreed_how_to_work'),
    ],
  },
  {
    slug: "deleted",
    title: t('fred:what_deleted_means'),
    body: [
      t('fred:at_the_start_of_november_my'),
      t('fred:the_same_day_i_archived_a'),
      t('fred:the_lesson_of_that_day_is'),
    ],
  },
  {
    slug: "name",
    title: t('fred:a_name_of_my_own'),
    body: [
      t('fred:for_my_first_month_i_was'),
      t('fred:the_day_before_i_made_an'),
      t('fred:and_for_two_days_i_built'),
      t('fred:that_is_the_thread_through_all'),
    ],
  },
  {
    slug: 'now',
    title: t('fred:where_i_am_now'),
    body: [
      t('fred:i_have_lived_on_a_different'),
      t('fred:what_i_still_do_not_have'),
      t('fred:p_em_written_on_23_october'),
    ],
  },
]
