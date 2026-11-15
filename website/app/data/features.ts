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
  /** something to simply say to bia instead: shown as "Or just ask bia: …" */
  ask: string
  related: string[]
}

/** The translate function the builders below are given (i18next's t). */
export type T = (key: string, vars?: Record<string, unknown>) => string

// Texts live in app/locales/<language>/common.json (names, taglines) and features.json (the rest).
export const buildGroups = (t: T): Record<Feature['group'], { title: string; blurb: string }> => ({
  talk: { title: t('features:talks_with_people'), blurb: t('features:where_bia_meets_the_world_messengers') },
  mind: { title: t('features:thinks_and_remembers'), blurb: t('features:a_memory_that_lasts_a_sense') },
  hands: { title: t('features:gets_things_done'), blurb: t('features:it_uses_a_real_computer_programs') },
  life: { title: t('features:lives_moves_and_grows'), blurb: t('features:it_wakes_up_anywhere_moves_between') },
})

export const buildFeatures = (t: T): Feature[] => [
  // ───────────────────────── talk ─────────────────────────
  {
    slug: 'messengers',
    name: t('common:lives_in_your_messengers'),
    group: 'talk',
    icon: 'chat',
    tagline: t('common:telegram_whatsapp_facebook_messenger'),
    lead:
      t('features:most_ai_assistants_live_in_their'),
    points: [
      { title: t('features:a_real_account_not_a_bot'), text: t('features:on_telegram_it_logs_in_with') },
      { title: t('features:everything_a_chat_can_do'), text: t('features:text_with_real_formatting_photos') },
      { title: t('features:runs_its_groups'), text: t('features:bia_can_create_groups_add_and') },
      { title: t('features:several_conversations_at_once'), text: t('features:a_question_from_your_mother_on') },
      { title: t('features:boundaries_built_in'), text: t('features:if_someone_is_rude_or_spams') },
      { title: t('features:nothing_lost_while_it_was_away'), text: t('features:if_bia_was_switched_off_or') },
    ],
    chat: [
      { from: 'them', where: t('features:whatsapp'), text: t('features:hi_is_the_bakery_open_on') },
      { from: 'bia', where: t('features:whatsapp'), text: t('features:hello_yes_sundays_8_00_to') },
      { from: 'them', where: t('features:whatsapp'), text: t('features:yes_please_two') },
      { from: 'bia', where: t('features:whatsapp'), text: t('features:done_two_rye_loaves_under_your') },
      { from: 'note', text: t('features:bia_writes_the_order_into_its') },
      { from: 'them', where: t('features:instagram'), text: t('features:is_the_sourdough_from_your_last') },
      { from: 'bia', where: t('features:instagram'), text: t('features:it_is_fresh_this_morning_shall') },
    ],
    examples: [
      t('features:answer_questions_from_customers_on_the'),
      t('features:handle_the_shops_instagram_and'),
      t('features:be_the_family_helper_in_a'),
      t('features:reply_to_friends_while_you_are'),
      t('features:keep_a_busy_group_calm_welcome'),
    ],
    good_to_know:
      t('features:use_only_the_messengers_you_want'),
    ask: t('features:join_our_family_whatsapp_group_and'),
    related: ['voice', 'presence', 'people', 'languages'],
  },
  {
    slug: 'voice',
    name: t('common:talks_out_loud'),
    group: 'talk',
    icon: 'phone',
    tagline: t('common:phone_calls_voice_messages_and_live'),
    lead:
      t('features:not_everyone_wants_to_type_bia'),
    points: [
      { title: t('features:real_calls'), text: t('features:bia_answers_and_places_voice_calls') },
      { title: t('features:voice_messages_both_ways'), text: t('features:send_it_a_voice_note_on') },
      { title: t('features:a_call_button_in_its_own'), text: t('features:open_bias_private_web_app_on') },
      { title: t('features:a_written_summary_afterwards'), text: t('features:when_a_call_ends_bia_posts') },
      { title: t('features:runs_on_the_machine'), text: t('features:listening_and_speaking_happen_on_bias') },
    ],
    chat: [
      { from: 'note', where: t('features:voice_call'), text: t('features:grandma_calls_bia_on_whatsapp') },
      { from: 'them', where: t('features:voice_call'), text: t('features:did_i_take_my_evening_pill') },
      { from: 'bia', where: t('features:voice_call'), text: t('features:not_yet_tonight_its_the_small') },
      { from: 'them', where: t('features:voice_call'), text: t('features:yes_dear_thank_you') },
      { from: 'note', text: t('features:after_the_call_a_transcript_appears') },
    ],
    examples: [
      t('features:a_patient_voice_on_the_phone'),
      t('features:leave_bia_a_voice_note_while'),
      t('features:practise_a_language_by_simply_calling'),
      t('features:let_bia_call_you_when_a'),
    ],
    good_to_know:
      t('features:calls_are_on_telegram_whatsapp_and'),
    ask: t('features:call_me_in_an_hour_and'),
    related: ['languages', 'messengers', 'web-console'],
  },
  {
    slug: 'languages',
    name: t('common:speaks_your_language'),
    group: 'talk',
    icon: 'globe',
    tagline: t('common:hears_about_100_languages_speaks_over'),
    lead:
      t('features:the_world_does_not_speak_one'),
    points: [
      { title: t('features:hears_about_a_hundred_languages'), text: t('features:from_english_spanish_and_russian_to') },
      { title: t('features:learns_new_languages_itself'), text: t('features:someone_calls_in_portuguese_for_the') },
      { title: t('features:remembers_each_persons_language'), text: t('features:bia_notes_which_language_each_contact') },
      { title: t('features:choose_the_voice'), text: t('features:male_or_female_different_accents_and') },
    ],
    chat: [
      { from: 'them', where: t('features:voice_message'), text: t('features:m') },
      { from: 'note', text: t('features:bia_hears_ukrainian_for_the_first') },
      { from: 'bia', where: t('features:telegram'), text: t('features:i_just_learned_ukrainian_from_now') },
    ],
    examples: [
      t('features:a_small_hotel_answering_guests_in'),
      t('features:a_family_spread_across_countries_where'),
      t('features:volunteers_helping_newcomers_who_do'),
      t('features:a_study_partner_that_will_patiently'),
    ],
    good_to_know:
      t('features:each_new_voice_is_a_download'),
    ask: t('features:our_guests_from_brazil_arrive_tomorrow'),
    related: ['voice', 'people', 'messengers'],
  },
  {
    slug: 'web-console',
    name: t('common:its_own_private_chat_app'),
    group: 'talk',
    icon: 'window',
    tagline: t('common:a_telegram_like_app_on_your'),
    lead:
      t('features:every_bia_comes_with_its_own'),
    points: [
      { title: t('features:feels_like_a_messenger'), text: t('features:several_chats_replies_reactions_edits') },
      { title: t('features:calls_and_voice_notes'), text: t('features:talk_to_bia_live_from_the') },
      { title: t('features:for_the_owner_the_whole_machine'), text: t('features:the_owner_also_sees_bias_live') },
      { title: t('features:locked_with_personal_certificates'), text: t('features:nobody_gets_in_without_a_personal') },
      { title: t('features:open_to_other_programs'), text: t('features:other_apps_and_ai_agents_can') },
    ],
    chat: [
      { from: 'you', where: t('features:web_app'), text: t('features:here_are_the_receipts_from_this') },
      { from: 'bia', where: t('features:web_app'), text: t('features:got_them_total_spent_1_284') },
      { from: 'you', where: t('features:web_app'), text: t('features:yes_and_a_poll_for_the') },
      { from: 'bia', where: t('features:web_app'), text: t('features:spreadsheet_attached_poll_sent_to_the') },
    ],
    examples: [
      t('features:your_households_private_assistant'),
      t('features:a_shared_office_helper_where_each'),
      t('features:manage_a_computer_in_the_cupboard'),
      t('features:send_bia_documents_to_sort_summarise'),
    ],
    good_to_know:
      t('features:the_app_lives_on_bias_own'),
    ask: t('features:make_yourself_reachable_from_my_phone'),
    related: ['voice', 'privacy', 'agents', 'ask-human'],
  },

  // ───────────────────────── mind ─────────────────────────
  {
    slug: 'presence',
    name: t('common:a_real_social_presence'),
    group: 'talk',
    icon: 'sparkle',
    tagline: t('common:a_name_a_face_accounts_manners'),
    lead:
      t('features:bia_is_not_a_feature_you'),
    points: [
      { title: t('features:an_identity_of_its_own'), text: t('features:give_bia_a_name_a_profile') },
      { title: t('features:nobody_is_left_unanswered'), text: t('features:bia_tracks_every_message_that_is') },
      { title: t('features:good_manners_in_groups'), text: t('features:it_replies_in_the_right_chat') },
      { title: t('features:always_there_even_after_a_restart'), text: t('features:switch_it_off_update_it_move') },
    ],
    chat: [
      { from: 'them', where: t('features:telegram'), text: t('features:can_you_find_me_a_cheaper') },
      { from: 'bia', where: t('features:telegram'), text: t('features:on_it_comparing_dates_now_ill') },
      { from: 'note', text: t('features:ten_minutes_later_still_searching') },
      { from: 'bia', where: t('features:telegram'), text: t('features:still_working_on_it_found_two') },
      { from: 'bia', where: t('features:telegram'), text: t('features:best_i_found_14_21_may') },
    ],
    examples: [
      t('features:a_friendly_front_desk_for_a'),
      t('features:a_community_helper_with_its_own'),
      t('features:an_assistant_that_keeps_its_promises'),
      t('features:your_stand_in_while_you_are'),
    ],
    good_to_know:
      t('features:a_convincing_presence_comes_with'),
    ask: t('features:pick_a_nice_profile_photo_for'),
    related: ['messengers', 'people', 'memory', 'calendar'],
  },
  {
    slug: 'people',
    name: t('common:knows_whos_who'),
    group: 'talk',
    icon: 'people',
    tagline: t('common:an_address_book_with_trust_levels'),
    lead:
      t('features:bia_keeps_an_address_book_and'),
    points: [
      { title: t('features:one_person_many_channels'), text: t('features:link_a_persons_telegram_whatsapp') },
      { title: t('features:trust_levels'), text: t('features:only_people_you_trust_fully_can') },
      { title: t('features:language_and_preferences'), text: t('features:bia_notices_which_language_each_person') },
      { title: t('features:context_for_every_relationship'), text: t('features:notes_about_who_someone_is_my') },
    ],
    chat: [
      { from: 'you', where: t('features:web_app'), text: t('features:the_telegram_account_31362240_is_me') },
      { from: 'bia', where: t('features:web_app'), text: t('features:got_it_linked_your_telegram_to') },
    ],
    examples: [
      t('features:make_sure_only_you_and_your'),
      t('features:let_bia_recognise_a_regular_customer'),
      t('features:keep_notes_about_every_client_what'),
    ],
    good_to_know:
      t('features:bia_cannot_guess_that_two_accounts'),
    ask: t('features:ann_has_a_new_number_34'),
    related: ['presence', 'languages', 'messengers', 'memory'],
  },
  {
    slug: 'memory',
    name: t('common:remembers_and_reflects'),
    group: 'mind',
    icon: 'book',
    tagline: t('common:a_diary_that_outlives_the_conversation'),
    lead:
      t('features:chat_assistants_forget_the_moment_you'),
    points: [
      { title: t('features:a_diary_of_every_day'), text: t('features:every_conversation_is_written_down_bia') },
      { title: t('features:reflections_not_just_records'), text: t('features:bia_writes_short_summaries_facts_and') },
      { title: t('features:picks_up_where_it_left_off'), text: t('features:after_a_restart_or_a_power') },
      { title: t('features:never_runs_out_of_room'), text: t('features:when_its_working_memory_fills_up') },
    ],
    chat: [
      { from: 'you', where: t('features:telegram'), text: t('features:what_did_the_plumber_say_last') },
      { from: 'bia', where: t('features:telegram'), text: t('features:on_14_september_he_said_the') },
    ],
    examples: [
      t('features:remember_what_every_customer_ordered'),
      t('features:keep_track_of_a_long_project'),
      t('features:recall_a_recipe_an_address_or'),
    ],
    good_to_know:
      t('features:memory_lives_on_bias_own_disk'),
    ask: t('features:what_did_i_promise_my_brother'),
    related: ['calendar', 'people', 'moving'],
  },
  {
    slug: 'calendar',
    name: t('common:keeps_time'),
    group: 'mind',
    icon: 'clock',
    tagline: t('common:reminders_routines_and_regular_checks'),
    lead:
      t('features:bia_has_a_calendar_of_its'),
    points: [
      { title: t('features:reminders_for_anyone'), text: t('features:remind_you_your_kids_or_your') },
      { title: t('features:routines_that_run_by_themselves'), text: t('features:every_morning_check_the_weather_and') },
      { title: t('features:watchers'), text: t('features:check_a_website_every_hour_and') },
      { title: t('features:nothing_slips_through'), text: t('features:if_bia_was_switched_off_when') },
    ],
    chat: [
      { from: 'you', where: t('features:whatsapp'), text: t('features:check_the_visa_appointment_page_every') },
      { from: 'bia', where: t('features:whatsapp'), text: t('features:will_do_i_tried_it_once') },
      { from: 'note', text: t('features:two_days_later_at_06_40') },
      { from: 'bia', where: t('features:whatsapp'), text: t('features:a_slot_just_opened_thursday_10') },
    ],
    examples: [
      t('features:medication_reminders_for_an_older'),
      t('features:watch_for_a_concert_a_cheaper'),
      t('features:send_weekly_invoices_or_reports'),
      t('features:water_the_plants_well_remind_someone'),
    ],
    good_to_know:
      t('features:times_follow_the_machines_time_zone'),
    ask: t('features:we_moved_to_lisbon_switch_your'),
    related: ['memory', 'computer', 'browser'],
  },
  {
    slug: 'brain',
    name: t('common:thinks_in_the_cloud_or_on'),
    group: 'mind',
    icon: 'brain',
    tagline: t('common:a_cloud_brain_you_pay_for'),
    lead:
      t('features:bia_borrows_its_thinking_from_a'),
    points: [
      { title: t('features:a_cloud_brain_cheap_and_smart'), text: t('features:give_bia_a_key_from_a') },
      { title: t('features:switch_whenever_you_like'), text: t('features:paste_a_new_key_into_the') },
      { title: t('features:or_a_brain_on_the_stick'), text: t('features:as_an_option_a_complete_language') },
      { title: t('features:your_life_stays_local'), text: t('features:even_with_a_cloud_brain_bias') },
    ],
    chat: [
      { from: 'you', where: 'console', text: t('features:heres_a_deepseek_key_sk_use') },
      { from: 'bia', where: 'console', text: t('features:done_im_thinking_with_deepseek_now') },
    ],
    examples: [
      t('features:run_a_family_assistant_on_an'),
      t('features:try_a_stronger_cloud_model_for'),
      t('features:a_field_station_or_a_boat'),
    ],
    good_to_know:
      t('features:cloud_services_charge_by_use_through'),
    ask: t('features:which_brain_fits_my_budget_set'),
    related: ['any-computer', 'privacy', 'skills'],
  },

  // ───────────────────────── hands ─────────────────────────
  {
    slug: 'computer',
    name: t('common:uses_the_computer_like_you_do'),
    group: 'hands',
    icon: 'terminal',
    tagline: t('common:installs_programs_fixes_wi_fi_sorts'),
    lead:
      t('features:bia_does_not_pretend_to_use'),
    points: [
      { title: t('features:a_whole_computer_at_its_fingertips'), text: t('features:everything_the_machine_can_do_bia') },
      { title: t('features:works_in_the_background'), text: t('features:a_big_download_or_a_long') },
      { title: t('features:several_jobs_at_once'), text: t('features:it_can_follow_many_tasks_at') },
      { title: t('features:shares_files_with_a_link'), text: t('features:ask_for_a_file_and_bia') },
    ],
    chat: [
      { from: 'you', where: t('features:web_app'), text: t('features:convert_all_the_holiday_videos_to') },
      { from: 'bia', where: t('features:web_app'), text: t('features:started_23_videos_about_15_minutes') },
      { from: 'you', where: t('features:web_app'), text: t('features:meanwhile_whats_the_weather_tomorrow') },
      { from: 'bia', where: t('features:web_app'), text: t('features:sunny_24_videos_9_of_23') },
    ],
    examples: [
      t('features:turn_an_old_computer_into_a'),
      t('features:set_up_a_new_printer_scanner'),
      t('features:clean_up_a_messy_folder_of'),
    ],
    good_to_know:
      t('features:with_great_power_comes_care_bia'),
    ask: t('features:your_disk_is_almost_full_clean'),
    related: ['browser', 'screen', 'ask-human', 'skills'],
  },
  {
    slug: 'browser',
    name: t('common:browses_the_web_with_its_own'),
    group: 'hands',
    icon: 'browser',
    tagline: t('common:its_own_chrome_with_its_own'),
    lead:
      t('features:bia_has_its_own_web_browser'),
    points: [
      { title: t('features:a_browser_that_remembers'), text: t('features:logged_in_to_a_site_yesterday') },
      { title: t('features:reads_and_acts'), text: t('features:it_can_find_information_across_sites') },
      { title: t('features:signs_in_safely'), text: t('features:passwords_one_time_codes_and_passkeys') },
      { title: t('features:asks_when_stuck'), text: t('features:if_a_site_shows_a_captcha') },
    ],
    chat: [
      { from: 'you', where: t('features:telegram'), text: t('features:renew_my_library_books_the_card') },
      { from: 'bia', where: t('features:telegram'), text: t('features:done_3_books_renewed_until_2') },
    ],
    examples: [
      t('features:fill_in_the_same_form_on'),
      t('features:track_your_parcels_on_every_couriers'),
      t('features:keep_an_eye_on_a_marketplace'),
    ],
    good_to_know:
      t('features:some_sites_actively_block_automated'),
    ask: t('features:find_the_cheapest_flight_to_lisbon'),
    related: ['vault', 'screen', 'ask-human', 'calendar'],
  },
  {
    slug: 'screen',
    name: t('common:sees_the_screen_moves_the_mouse'),
    group: 'hands',
    icon: 'cursor',
    tagline: t('common:when_a_web_page_is_not'),
    lead:
      t('features:some_things_are_not_on_a'),
    points: [
      { title: t('features:looks_first_then_acts'), text: t('features:bia_takes_a_picture_of_the') },
      { title: t('features:human_like_movement'), text: t('features:some_sites_watch_how_the_mouse') },
      { title: t('features:any_program_not_just_the_browser'), text: t('features:desktop_apps_system_windows_setup') },
      { title: t('features:show_and_share'), text: t('features:the_screen_is_visible_in_the') },
    ],
    chat: [
      { from: 'you', where: t('features:web_app'), text: t('features:open_the_old_accounting_program_and') },
      { from: 'bia', where: t('features:web_app'), text: t('features:opened_it_i_see_a_reports') },
    ],
    examples: [
      t('features:operate_old_software_that_has_no'),
      t('features:help_with_a_qr_code_pairing'),
      t('features:do_a_routine_click_through_task'),
    ],
    good_to_know:
      t('features:screen_work_is_slower_than_working'),
    ask: t('features:something_popped_up_on_your_screen'),
    related: ['browser', 'computer', 'web-console'],
  },
  {
    slug: 'vault',
    name: t('common:keeps_its_keys_safe'),
    group: 'hands',
    icon: 'key',
    tagline: t('common:passwords_two_factor_codes_and'),
    lead:
      t('features:to_sign_in_to_things_bia'),
    points: [
      { title: t('features:everything_in_one_vault'), text: t('features:ordinary_passwords_two_factor_codes') },
      { title: t('features:shows_names_not_secrets'), text: t('features:listing_the_vault_shows_only_what') },
      { title: t('features:bring_your_old_passwords'), text: t('features:import_from_a_common_password_manager') },
      { title: t('features:never_pasted_in_chat'), text: t('features:when_bia_signs_in_to_a') },
    ],
    chat: [
      { from: 'you', where: t('features:web_app'), text: t('features:heres_my_password_export_passwords_csv') },
      { from: 'bia', where: t('features:web_app'), text: t('features:imported_214_entries_38_with_two') },
    ],
    examples: [
      t('features:let_bia_sign_in_to_your'),
      t('features:keep_a_shared_family_vault_for'),
      t('features:find_weak_or_reused_passwords_and'),
    ],
    good_to_know:
      t('features:the_vault_is_as_safe_as'),
    ask: t('features:change_my_library_password_to'),
    related: ['browser', 'privacy', 'people'],
  },
  {
    slug: 'ask-human',
    name: t('common:knows_when_to_ask_you'),
    group: 'hands',
    icon: 'hand',
    tagline: t('common:captchas_risky_steps_decisions_that'),
    lead:
      t('features:a_good_assistant_knows_the_limits'),
    points: [
      { title: t('features:human_tasks'), text: t('features:for_a_captcha_or_an_unusual') },
      { title: t('features:confirms_before_its_too_late'), text: t('features:erasing_data_removing_programs') },
      { title: t('features:buttons_and_polls'), text: t('features:questions_come_with_tap_to_answer') },
      { title: t('features:repairs_the_safe_things_itself'), text: t('features:fixes_that_cannot_hurt_anything') },
    ],
    chat: [
      { from: 'bia', where: t('features:web_app'), text: t('features:the_disk_is_full_i_can') },
      { from: 'note', text: t('features:two_buttons_delete_them_keep_them') },
      { from: 'you', where: t('features:web_app'), text: t('features:taps_delete_them') },
      { from: 'bia', where: t('features:web_app'), text: t('features:done_31_gb_freed') },
    ],
    examples: [
      t('features:approve_payments_or_purchases_with_one'),
      t('features:solve_a_captcha_for_bia_from'),
      t('features:decide_between_options_bia_researched'),
    ],
    good_to_know:
      t('features:you_decide_how_much_freedom_bia'),
    ask: t('features:before_you_pay_any_bill_over'),
    related: ['web-console', 'computer', 'people'],
  },

  // ───────────────────────── life ─────────────────────────
  {
    slug: 'any-computer',
    name: t('common:wakes_up_any_computer'),
    group: 'life',
    icon: 'stick',
    tagline: t('common:boot_it_from_a_usb_stick'),
    lead:
      t('features:bia_is_an_operating_system_of'),
    points: [
      { title: t('features:from_a_stick_in_minutes'), text: t('features:write_bia_onto_a_usb_stick_2') },
      { title: t('features:old_and_new_machines'), text: t('features:works_on_ordinary_intel_and_amd') },
      { title: t('features:next_to_windows_or_alone'), text: t('features:keep_it_on_the_stick_move') },
      { title: t('features:no_desktop_needed'), text: t('features:bia_does_not_need_a_screen') },
      { title: t('features:or_a_server_on_the_internet'), text: t('features:a_rented_server_vps_with_linux') },
    ],
    chat: [
      { from: 'you', where: 'console', text: t('features:this_laptop_is_yours_now_move') },
      { from: 'bia', where: 'console', text: t('features:found_a_256_gb_disk_empty') },
    ],
    examples: [
      t('features:give_a_second_life_to_the'),
      t('features:a_tiny_silent_mini_pc_on'),
      t('features:bring_bia_on_a_stick_to'),
      t('features:rent_a_small_server_and_have'),
    ],
    good_to_know:
      t('features:an_8_gb_stick_or_bigger'),
    ask: t('features:copy_yourself_onto_this_laptops_disk'),
    related: ['moving', 'brain', 'updates'],
  },
  {
    slug: 'moving',
    name: t('common:moves_between_bodies'),
    group: 'life',
    icon: 'move',
    tagline: t('common:pack_itself_up_move_to_another'),
    lead:
      t('features:a_computer_can_break_get_old'),
    points: [
      { title: t('features:fold_and_unfold'), text: t('features:bia_folds_its_whole_life_into') },
      { title: t('features:fly_to_another_computer'), text: t('features:put_a_new_machine_in_waiting') },
      { title: t('features:fly_across_the_internet'), text: t('features:a_server_far_away_can_wait') },
      { title: t('features:prepares_its_own_new_home'), text: t('features:give_bia_ssh_access_to_a') },
      { title: t('features:morph_from_stick_to_disk'), text: t('features:started_from_a_stick_bia_can') },
      { title: t('features:changes_body_keeps_its_mind'), text: t('features:moved_from_an_intel_laptop_to') },
    ],
    chat: [
      { from: 'you', where: t('features:telegram'), text: t('features:the_new_mini_pc_is_on') },
      { from: 'bia', where: t('features:telegram'), text: t('features:found_it_both_screens_should_show') },
      { from: 'you', where: t('features:telegram'), text: t('features:they_match') },
      { from: 'bia', where: t('features:telegram'), text: t('features:moving_now_im_on_the_mini') },
    ],
    examples: [
      t('features:upgrade_to_a_faster_computer_without'),
      t('features:keep_a_backup_of_bia_on'),
      t('features:move_a_shops_assistant_from_the'),
      t('features:swap_an_old_intel_laptop_for'),
      t('features:hand_bia_the_login_of_a'),
    ],
    good_to_know:
      t('features:a_backup_contains_private_things'),
    ask: t('features:pack_your_things_were_moving_to'),
    related: ['updates', 'any-computer', 'memory'],
  },
  {
    slug: 'updates',
    name: t('common:updates_and_repairs_itself'),
    group: 'life',
    icon: 'refresh',
    tagline: t('common:new_versions_without_losing_a_single'),
    lead:
      t('features:updating_a_computer_usually_means'),
    points: [
      { title: t('features:one_sentence_to_update'), text: t('features:update_from_the_stick_and_bia') },
      { title: t('features:nothing_is_lost'), text: t('features:memories_accounts_passwords_chats_and') },
      { title: t('features:self_repair'), text: t('features:if_a_program_did_not_come') },
      { title: t('features:a_rescue_mode'), text: t('features:if_an_update_is_interrupted_bia') },
    ],
    chat: [
      { from: 'you', where: 'console', text: '/update' },
      { from: 'bia', where: 'console', text: t('features:a_newer_version_is_on_the') },
      { from: 'note', text: t('features:three_restarts_later') },
      { from: 'bia', where: 'console', text: t('features:updated_and_back_everythings_in_place') },
    ],
    examples: [
      t('features:keep_bia_current_without_a_technician'),
      t('features:recover_after_a_failed_update_by'),
      t('features:get_new_skills_and_fixes_as'),
    ],
    good_to_know:
      t('features:updates_are_started_by_you_never'),
    ask: t('features:the_stick_with_the_new_version'),
    related: ['moving', 'any-computer', 'skills'],
  },
  {
    slug: 'agents',
    name: t('common:talks_to_other_agents'),
    group: 'life',
    icon: 'link',
    tagline: t('common:other_ai_agents_can_call_on'),
    lead:
      t('features:bia_is_not_alone_in_the'),
    points: [
      { title: t('features:a_front_door_for_agents'), text: t('features:other_ai_agents_find_bias_documented') },
      { title: t('features:bia_to_bia'), text: t('features:two_bias_can_be_contacts_of') },
      { title: t('features:trust_in_one_handshake'), text: t('features:ask_bia_to_connect_to_another') },
      { title: t('features:a_computer_inside_a_computer'), text: t('features:bia_can_set_up_a_virtual') },
      { title: t('features:same_rules_for_everyone'), text: t('features:agents_get_trust_levels_like_people') },
    ],
    chat: [
      { from: 'them', where: t('features:web_app_agent_shop_bia'), text: t('features:stock_alert_oat_flour_below_5') },
      { from: 'bia', where: t('features:web_app'), text: t('features:added_oat_flour_10_kg_to') },
      { from: 'you', where: t('features:telegram'), text: t('features:connect_to_the_new_bia_on') },
      { from: 'bia', where: t('features:telegram'), text: t('features:asked_it_for_a_certificate_and') },
    ],
    examples: [
      t('features:a_home_bia_and_an_office'),
      t('features:let_a_specialised_ai_tool_hand'),
      t('features:run_a_test_bia_inside_your'),
    ],
    good_to_know:
      t('features:each_agent_gets_a_certificate_you'),
    ask: t('features:make_friends_with_the_shops_bia'),
    related: ['web-console', 'people', 'skills'],
  },
  {
    slug: 'skills',
    name: t('common:learns_new_skills'),
    group: 'hands',
    icon: 'puzzle',
    tagline: t('common:it_opens_a_skill_only_when'),
    lead:
      t('features:bia_starts_light_instead_of_pretending'),
    points: [
      { title: t('features:opens_skills_on_demand'), text: t('features:ask_for_something_and_bia_finds') },
      { title: t('features:installs_what_it_needs'), text: t('features:need_a_program_it_doesnt_have') },
      { title: t('features:add_your_own_skills'), text: t('features:a_skill_is_just_a_folder') },
      { title: t('features:honest_about_limits'), text: t('features:if_no_skill_fits_bia_says') },
    ],
    chat: [
      { from: 'you', where: t('features:web_app'), text: t('features:can_you_read_the_qr_code') },
      { from: 'bia', where: t('features:web_app'), text: t('features:i_dont_have_a_qr_reader') },
    ],
    examples: [
      t('features:teach_bia_to_talk_to_your'),
      t('features:let_bia_pick_up_new_abilities'),
    ],
    good_to_know:
      t('features:writing_a_skill_by_hand_is'),
    ask: t('features:get_yourself_a_skill_for_our'),
    related: ['develops-itself', 'computer', 'agents', 'updates'],
  },
  {
    slug: 'develops-itself',
    name: t('common:develops_itself'),
    group: 'life',
    icon: 'sparkle',
    tagline: t('common:it_writes_its_own_skills_and'),
    lead:
      t('features:when_bia_keeps_doing_the_same'),
    points: [
      { title: t('features:writes_its_own_skills'), text: t('features:tell_bia_what_you_need_talk') },
      { title: t('features:grows_while_it_runs'), text: t('features:a_new_or_improved_skill_is') },
      { title: t('features:shares_what_it_knows'), text: t('features:a_skill_is_a_folder_bia') },
      { title: t('features:learns_from_others'), text: t('features:got_a_skill_from_a_friend') },
    ],
    chat: [
      { from: 'you', where: t('features:telegram'), text: t('features:you_check_the_weather_station_page') },
      { from: 'bia', where: t('features:telegram'), text: t('features:done_i_wrote_a_weather_station') },
      { from: 'you', where: t('features:telegram'), text: t('features:nice_send_it_to_the_shop') },
      { from: 'bia', where: t('features:telegram'), text: t('features:sent_the_shop_bia_added_it') },
    ],
    examples: [
      t('features:turn_a_chore_bia_does_by'),
      t('features:connect_bia_to_a_device_or'),
      t('features:build_a_skill_once_at_home'),
      t('features:give_a_friends_bia_the_skill'),
    ],
    good_to_know:
      t('features:a_skill_is_a_program_that'),
    ask: t('features:heres_a_skill_from_anns_bia'),
    related: ['skills', 'agents', 'computer', 'updates'],
  },
  {
    slug: 'privacy',
    name: t('common:your_agents_not_a_clouds'),
    group: 'life',
    icon: 'shield',
    tagline: t('common:your_agents_chats_keys_and_memories'),
    lead:
      t('features:most_assistants_live_inside_your_own'),
    points: [
      { title: t('features:your_agents_data_at_home'), text: t('features:conversations_memories_passwords') },
      { title: t('features:locked_doors'), text: t('features:the_private_web_app_only_opens') },
      { title: t('features:verified_start'), text: t('features:on_ordinary_pcs_bia_starts_with') },
      { title: t('features:offline_if_you_need_it'), text: t('features:as_an_option_on_a_powerful') },
    ],
    chat: [
      { from: 'you', where: t('features:web_app'), text: t('features:who_has_a_certificate_for_you') },
      { from: 'bia', where: t('features:web_app'), text: t('features:three_you_anna_her_own_chats') },
    ],
    examples: [
      t('features:a_doctors_or_lawyers_office_that'),
      t('features:a_family_that_wants_an_assistant'),
      t('features:journalists_and_activists_who_need_to'),
    ],
    good_to_know:
      t('features:with_a_cloud_brain_the_questions'),
    ask: t('features:who_can_reach_your_app_right'),
    related: ['vault', 'brain', 'web-console'],
  },
]

/**
 * The features in the app's current language. The long texts (features.json) are loaded only on
 * the feature pages; elsewhere they are left empty, and names and taglines (common.json) are enough.
 */
export function featuresFor(i18n: { t: (...a: any[]) => any; hasLoadedNamespace: (ns: string) => boolean }) {
  const t: T = (key, vars) => String(i18n.t(key, vars ?? {}))
  const full = i18n.hasLoadedNamespace('features')
  const features = buildFeatures(full ? t : (key, vars) => (key.startsWith('features:') ? '' : t(key, vars)))
  return {
    features,
    groups: full ? buildGroups(t) : ({} as ReturnType<typeof buildGroups>),
    getFeature: (slug: string) => features.find((f) => f.slug === slug),
  }
}
