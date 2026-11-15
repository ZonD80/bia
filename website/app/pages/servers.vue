<template>
  <main>
    <section class="page-hero">
      <div class="container">
        <nav class="crumbs" :aria-label="$t('servers:breadcrumb')">
          <NuxtLink :to="$lp('/')">{{ $t('servers:home') }}</NuxtLink><span aria-hidden="true">/</span>
          <NuxtLink :to="$lp('/download')">{{ $t('servers:get_bia') }}</NuxtLink><span aria-hidden="true">/</span><span>{{ $t('servers:servers_and_virtual_machines') }}</span>
        </nav>
        <span class="eyebrow">{{ $t('servers:for_servers_clouds_and_virtual') }}</span>
        <h1>{{ $t('servers:bia_on_a_server_or_a') }}</h1>
        <p class="section-lead">
          {{ $t('servers:a_rented_server_a_machine_in') }}
        </p>
        <p class="cloud-hint">
          <Tr k="servers:rather_not_rent_and_set_up"><NuxtLink :to="$lp('/cloud')" /></Tr>
        </p>
        <div class="grid grid--3 ways">
          <a v-for="w in ways" :key="w.id" class="card way" :href="`#${w.id}`">
            <span class="way__tag">{{ w.tag }}</span>
            <h3>{{ w.title }}</h3>
            <p>{{ w.text }}</p>
          </a>
        </div>
      </div>
    </section>

    <!-- ───────────── takeover ───────────── -->
    <section id="takeover" class="section anchor">
      <div class="container">
        <span class="eyebrow">{{ $t('servers:way_1_one_command') }}</span>
        <h2 class="h2 h2--top">{{ $t('servers:turn_a_running_linux_server_into') }}</h2>
        <p class="section-lead">
          {{ $t('servers:have_a_server_with_ubuntu_debian') }}
        </p>
        <pre class="screen screen--cmd">curl -fsSL https://github.com/zond80/bia/releases/latest/download/takeover.sh | sudo bash</pre>
        <div class="note-box tip">
          <p>
            <Tr k="servers:no_terminal_needed_bia_deploy_on"><strong /><NuxtLink :to="$lp('/download')" /></Tr>
          </p>
        </div>

        <h3 class="h3">{{ $t('servers:what_happens') }}</h3>
        <ol class="flow">
          <li v-for="(s, i) in takeoverSteps" :key="i" class="flow__item">
            <span class="flow__n">{{ i + 1 }}</span>
            <div>
              <h3>{{ s.title }}</h3>
              <p>{{ s.text }}</p>
              <pre v-if="s.screen" class="screen">{{ s.screen }}</pre>
            </div>
          </li>
        </ol>

        <h3 class="h3">{{ $t('servers:what_the_server_needs') }}</h3>
        <ul class="list-check">
          <li>
            <Tr k="servers:linux_on_intel_or_amd_x86"><strong /></Tr>
          </li>
          <li>
            <Tr k="servers:about_4_gb_of_memory_3"><strong /></Tr>
          </li>
          <li><Tr k="servers:a_disk_of_at_least_8"><strong /></Tr></li>
          <li><Tr k="servers:internet_access_and_an_api_key"><strong /></Tr></li>
          <li>
            <Tr k="servers:open_ports_443_for_bias_web"><strong /></Tr>
          </li>
          <li>
            <Tr k="servers:the_providers_console_just_in_case"><strong /></Tr>
          </li>
        </ul>
        <p class="aside">
          {{ $t('servers:before_you_confirm_nothing_on_the') }}
        </p>

        <h3 class="h3">{{ $t('servers:answers_from_a_file_or_a') }}</h3>
        <p>
          <Tr k="servers:for_several_servers_or_to_run"><code /><code /><code /></Tr>
        </p>
        <div class="grid grid--2 cfg">
          <div>
            <pre class="screen">{{ configExample }}</pre>
          </div>
          <div>
            <pre class="screen">{{ configCommand }}</pre>
            <ul class="cfg__list">
              <li><Tr k="servers:setup_instructions_for_bia_read_once"><code /></Tr></li>
              <li><Tr k="servers:pki_names_and_pki_ips_names"><code /><code /></Tr></li>
              <li><Tr k="servers:config_sha256_pins_a_link_to"><code /></Tr></li>
              <li><Tr k="servers:dump_config_shows_the_plan_without"><code /><code /></Tr></li>
            </ul>
          </div>
        </div>

        <h3 class="h3">{{ $t('servers:or_a_land_waiting_for_your') }}</h3>
        <p>
          <Tr k="servers:already_have_a_bia_with_its"><code /></Tr>
        </p>
        <ol class="flow">
          <li v-for="(s, i) in landSteps" :key="i" class="flow__item">
            <span class="flow__n">{{ i + 1 }}</span>
            <div>
              <h3>{{ s.title }}</h3>
              <p>{{ s.text }}</p>
              <pre v-if="s.screen" class="screen">{{ s.screen }}</pre>
            </div>
          </li>
        </ol>
        <p class="aside">
          {{ $t('servers:the_land_secret_stays_on_the') }}
        </p>
      </div>
    </section>

    <!-- ───────────── agent does it ───────────── -->
    <section id="by-bia" class="section section--tint anchor">
      <div class="container">
        <span class="eyebrow">{{ $t('servers:way_2_in_one_sentence') }}</span>
        <h2 class="h2 h2--top">{{ $t('servers:let_your_bia_set_up_the') }}</h2>
        <p class="section-lead">
          {{ $t('servers:bia_knows_how_to_do_all') }}
        </p>
        <div class="grid grid--2">
          <div class="chat card">
            <p v-for="(l, i) in agentChat" :key="i" class="chat__line" :class="`chat__line--${l.from}`">
              <span v-if="l.from !== 'note'" class="chat__who">{{ l.from === 'you' ? $t('servers:you') : 'bia' }}</span>
              {{ l.text }}
            </p>
          </div>
          <ul class="list-check">
            <li>
              <Tr k="servers:ssh_access_a_key_bia_can"><strong /></Tr>
            </li>
            <li>
              <Tr k="servers:another_bias_body_the_server_starts"><strong /></Tr>
            </li>
            <li>
              <Tr k="servers:the_same_rules_as_above_about"><strong /></Tr>
            </li>
            <li>
              <Tr k="servers:many_servers_give_bia_several"><strong /></Tr>
            </li>
          </ul>
        </div>
      </div>
    </section>

    <!-- ───────────── installer ───────────── -->
    <section id="installer" class="section anchor">
      <div class="container">
        <span class="eyebrow">{{ $t('servers:way_3_from_an_installer_image') }}</span>
        <h2 class="h2 h2--top">{{ $t('servers:install_on_an_empty_machine') }}</h2>
        <p class="section-lead">
          {{ $t('servers:for_a_new_virtual_machine_a') }}
        </p>

        <h3 class="h3">{{ $t('servers:1_download_the_installer') }}</h3>
        <div class="grid grid--2">
          <div v-for="a in arches" :key="a.id" class="card dl">
            <h3>{{ a.label }}</h3>
            <p class="dl__note">{{ a.note }}</p>
            <a
              class="btn btn--primary dl__btn"
              :class="{ 'is-busy': loading }"
              :href="landDownloads[a.id].url"
              :aria-busy="loading"
            >
              {{ buttonText(a.id) }}
            </a>
            <p class="dl__meta">
              <code>{{ LAND_ASSETS[a.id] }}</code>
              <template v-if="landDownloads[a.id].available">
                {{ $t('servers:version', { version: landDownloads[a.id].version }) }}
                <template v-if="landDownloads[a.id].size"> · {{ mib(landDownloads[a.id].size) }}</template>
              </template>
            </p>
          </div>
        </div>
        <div class="note-box tip">
          <p>
            <Tr k="servers:the_download_is_compressed_iso_xz"><strong /><code /><code /></Tr>
          </p>
          <ul>
            <li><Tr k="servers:mac_double_click_the_file_or"><strong /><code /></Tr></li>
            <li><strong>{{ $t('servers:linux') }}</strong> <code>xz -dk bia-land-x86_64.iso.xz</code></li>
            <li><Tr k="servers:windows_open_it_with_7_zip"><strong /><a href="https://www.7-zip.org/" /></Tr></li>
            <li>
              <Tr k="servers:writing_a_usb_stick_no_need"><strong /><code /></Tr>
            </li>
          </ul>
        </div>

        <h3 class="h3">{{ $t('servers:2_what_the_machine_needs') }}</h3>
        <ul class="list-check">
          <li>
            <Tr k="servers:modern_uefi_start_up_the_installer"><strong /><a href="#takeover" /></Tr>
          </li>
          <li>
            <Tr k="servers:a_disk_of_at_least_8_2"><strong /></Tr>
          </li>
          <li><Tr k="servers:memory_and_processor_we_test_with"><strong /></Tr></li>
          <li>
            <Tr k="servers:a_console_for_the_first_minutes"><strong /></Tr>
          </li>
          <li>
            <Tr k="servers:internet_access_after_installing_the"><strong /></Tr>
          </li>
          <li>
            <Tr k="servers:a_bia_that_can_reach_it"><strong /></Tr>
          </li>
        </ul>
        <p class="aside">
          {{ $t('servers:the_server_gets_no_ai_model') }}
        </p>

        <h3 class="h3">{{ $t('servers:3_install_then_let_bia_fly') }}</h3>
        <ol class="flow">
          <li v-for="(s, i) in isoSteps" :key="i" class="flow__item">
            <span class="flow__n">{{ i + 1 }}</span>
            <div>
              <h3>{{ s.title }}</h3>
              <p>{{ s.text }}</p>
              <pre v-if="s.screen" class="screen">{{ s.screen }}</pre>
            </div>
          </li>
        </ol>
      </div>
    </section>

    <section class="section section--tint">
      <div class="container">
        <h2 class="h2">{{ $t('servers:virtual_machine_settings') }}</h2>
        <p class="section-lead">
          <Tr k="servers:any_hypervisor_works_if_the_virtual"><code /><a href="#takeover" /></Tr>
        </p>
        <div class="grid grid--3">
          <div v-for="h in hypervisors" :key="h.name" class="card">
            <h3>{{ h.name }}</h3>
            <p>{{ h.text }}</p>
          </div>
        </div>
        <p class="aside">
          {{ $t('servers:our_automated_tests_run_the_installer') }}
        </p>

        <h2 class="h2">{{ $t('servers:physical_servers_and_clouds') }}</h2>
        <div class="grid grid--3">
          <div v-for="p in places" :key="p.name" class="card">
            <h3>{{ p.name }}</h3>
            <p>{{ p.text }}</p>
          </div>
        </div>
      </div>
    </section>

    <section class="section">
      <div class="container">
        <h2 class="h2">{{ $t('servers:good_to_know') }}</h2>
        <ul class="list-check">
          <li v-for="g in goodToKnow" :key="g.title">
            <strong>{{ g.title }}</strong> {{ g.text }}
          </li>
        </ul>
      </div>
    </section>

    <CtaBand :title="$t('servers:prefer_a_usb_stick')" :text="$t('servers:for_a_computer_on_your_desk')" />
  </main>
</template>

<script setup lang="ts">
const t = useT()
import { LAND_ASSETS, type LandArch } from '~/composables/useLatestRelease'


useSeoMeta({
  title: t('servers:bia_on_servers_vps_and_virtual'),
  description:
    t('servers:turn_a_running_linux_server_or'),
})

const ways = [
  {
    id: 'takeover',
    tag: t('servers:running_linux'),
    title: t('servers:one_command'),
    text: t('servers:run_it_on_a_rented_server'),
  },
  {
    id: 'by-bia',
    tag: t('servers:you_have_a_bia'),
    title: t('servers:ask_your_bia'),
    text: t('servers:give_it_ssh_access_to_a'),
  },
  {
    id: 'installer',
    tag: t('servers:empty_machine'),
    title: t('servers:installer_image'),
    text: t('servers:start_a_new_virtual_machine_or'),
  },
]

const takeoverSteps = [
  {
    title: t('servers:answer_a_few_questions'),
    text: t('servers:the_api_key_for_bias_cloud'),
    screen: t('servers:this_machine_x86_64_uefi_4096'),
  },
  {
    title: t('servers:check_the_plan_and_confirm'),
    text: t('servers:you_see_everything_that_will_happen'),
  },
  {
    title: t('servers:download_your_personal_certificate'),
    text: t('servers:the_command_makes_bias_certificates'),
  },
  {
    title: t('servers:the_server_restarts_into_bia'),
    text: t('servers:bia_writes_itself_onto_the_disk'),
  },
  {
    title: t('servers:open_bia'),
    text: t('servers:go_to_https_the_servers_address'),
  },
]

const landSteps = [
  {
    title: t('servers:get_a_land_secret'),
    text: t('servers:ask_your_bia_for_a_new'),
  },
  {
    title: t('servers:take_the_server_over_in_land'),
    text: t('servers:the_secret_goes_along_the_server'),
    screen: t('servers:curl_fssl_https_github_com_zond80'),
  },
  {
    title: t('servers:tell_your_bia_to_fly'),
    text: t('servers:fly_to_203_0_113_7'),
  },
]

const agentChat = [
  { from: 'you', text: t('servers:here_is_a_new_server_root') },
  { from: 'bia', text: t('servers:made_a_land_secret_for_203') },
  { from: 'note', text: t('servers:five_minutes_later') },
  { from: 'bia', text: t('servers:the_server_restarted_and_is_waiting') },
]

const configExample = `{
  "fmm": { "key": "sk-…" },
  "lang": "de",
  "setup": "Connect to the bia at https://203.0.113.5 and set up trust with it.",
  "pki": { "names": ["bia.example.com"] }
}`

const configCommand = `curl -fsSL …/takeover.sh | sudo bash -s -- \\
  --config https://example.com/bia.json \\
  --wipe /dev/vda --yes --reboot`

const arches: { id: LandArch; label: string; note: string }[] = [
  { id: 'x86_64', label: t('servers:intel_and_amd_x86_64'), note: t('servers:most_servers_cloud_machines_and') },
  { id: 'arm64', label: t('servers:arm_arm64'), note: t('servers:arm_servers_and_cloud_machines_and') },
]

const isoSteps = [
  {
    title: t('servers:start_the_machine_from_the_installer'),
    text: t('servers:attach_the_iso_as_the_virtual'),
  },
  {
    title: t('servers:set_up_the_network'),
    text: t('servers:pick_the_network_interface_if_there'),
    screen: t('servers:use_dhcp_yes_no_no_address'),
  },
  {
    title: t('servers:choose_the_disk'),
    text: t('servers:the_installer_lists_the_disks_it'),
  },
  {
    title: t('servers:eject_the_iso_and_restart'),
    text: t('servers:remove_the_iso_from_the_drive'),
  },
  {
    title: t('servers:ask_your_bia_to_fly'),
    text: t('servers:tell_an_existing_bia_for_example'),
  },
  {
    title: t('servers:the_server_wakes_up_as_bia'),
    text: t('servers:the_server_restarts_with_the_settings'),
  },
]

const hypervisors = [
  {
    name: 'Proxmox VE',
    text: t('servers:bios_ovmf_uefi_with_an_efi'),
  },
  {
    name: 'VMware',
    text: t('servers:esxi_workstation_and_fusion_set_the'),
  },
  {
    name: 'VirtualBox',
    text: t('servers:settings_system_turn_on_enable_efi'),
  },
  {
    name: 'Hyper-V',
    text: t('servers:create_a_generation_2_machine_set'),
  },
  {
    name: t('servers:hv_qemu'),
    text: t('servers:start_with_uefi_firmware_ovmf_for'),
  },
  {
    name: 'Apple Silicon Mac',
    text: t('servers:use_the_arm_installer_in_an'),
  },
]

const places = [
  {
    name: t('servers:place_vps'),
    text: t('servers:order_it_with_ubuntu_or_debian'),
  },
  {
    name: t('servers:place_console'),
    text: t('servers:run_the_one_command_if_it'),
  },
  {
    name: t('servers:place_iso'),
    text: t('servers:any_provider_that_lets_you_start'),
  },
]

const goodToKnow = [
  {
    title: t('servers:the_network_comes_along'),
    text: t('servers:the_one_command_copies_the_servers'),
  },
  {
    title: t('servers:your_certificate_is_the_only_way'),
    text: t('servers:bia_has_no_password_login_its'),
  },
  {
    title: t('servers:two_bias_one_handshake'),
    text: t('servers:setup_instructions_can_tell_a_new'),
  },
  {
    title: t('servers:flying_copies'),
    text: t('servers:the_bia_that_flies_keeps_running'),
  },
  {
    title: t('servers:many_servers_one_setup'),
    text: t('servers:prepare_one_bia_the_way_every'),
  },
  {
    title: t('servers:a_waiting_server_keeps_waiting'),
    text: t('servers:even_after_a_restart_or_an'),
  },
]

const { landDownloads, loading, fetched } = useLatestRelease()

function buttonText(id: LandArch): string {
  if (loading.value) return t('common:checking_latest')
  if (fetched.value && !landDownloads.value[id].available) return t('common:see_releases')
  return t('servers:download_installer')
}

function mib(bytes: number): string {
  return bytes >= 1 << 30 ? `${(bytes / (1 << 30)).toFixed(1)} GB` : `${Math.round(bytes / (1 << 20))} MB`
}
</script>

<style scoped>
.h2 {
  font-size: clamp(1.4rem, 3vw, 1.9rem);
  margin: 2.5rem 0 1rem;
}
.h2:first-child, .h2--top { margin-top: 0.3rem; }
.h3 { font-size: 1.25rem; margin: 2rem 0 0.75rem; }
.anchor { scroll-margin-top: calc(4.5rem + var(--strip-h, 0px)); }
.ways { margin-top: 1.5rem; }
.way { display: block; color: inherit; text-decoration: none; transition: transform 0.15s ease, border-color 0.15s ease; }
.way:hover { transform: translateY(-2px); border-color: var(--accent); }
.way h3 { font-size: 1.25rem; margin: 0.3rem 0; }
.way p { margin: 0; color: var(--ink-soft); }
.way__tag { font-size: 0.8rem; font-weight: 700; color: var(--accent); text-transform: uppercase; letter-spacing: 0.04em; }
.dl h3 { font-size: 1.3rem; margin-bottom: 0.2rem; }
.dl__note { color: var(--mute); }
.dl { display: flex; flex-direction: column; }
.dl__btn { width: 100%; margin-top: auto; }
.dl__btn.is-busy { opacity: 0.7; pointer-events: none; }
.dl__meta { margin: 0.5rem 0 0; font-size: 0.85rem; color: var(--mute); text-align: center; overflow-wrap: anywhere; }
.tip { margin-top: 1.25rem; }
.tip p { margin: 0; }
.tip ul { margin: 0.75rem 0 0; padding-left: 1.2rem; }
.tip li + li { margin-top: 0.35rem; }
.aside { margin: 1rem 0 0; color: var(--mute); }
.cfg { align-items: start; }
.cfg > div, .grid > * { min-width: 0; }
code { overflow-wrap: anywhere; }
.cfg .screen { margin-top: 0; }
.cfg__list { margin: 0.75rem 0 0; padding-left: 1.2rem; color: var(--ink-soft); }
.cfg__list li + li { margin-top: 0.35rem; }
.chat { display: grid; gap: 0.6rem; align-content: start; }
.chat__line { margin: 0; padding: 0.6rem 0.8rem; border-radius: 0.75rem; background: var(--bg, #fff); border: 1px solid var(--line); }
.chat__line--you { background: color-mix(in srgb, var(--accent) 10%, transparent); }
.chat__line--note { background: none; border: none; text-align: center; font-size: 0.85rem; color: var(--mute); font-style: italic; }
.chat__who { display: block; font-size: 0.75rem; font-weight: 700; color: var(--mute); margin-bottom: 0.15rem; }
.flow {
  list-style: none;
  margin: 1rem 0 0;
  padding: 0;
  display: grid;
  gap: 1.25rem;
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
  min-width: 0;
}
.flow__item > div { min-width: 0; }
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
.screen {
  margin: 0.75rem 0 0;
  padding: 0.75rem 1rem;
  background: #1d2126;
  color: #e8ece4;
  border-radius: 0.5rem;
  font-size: 0.85rem;
  line-height: 1.5;
  overflow-x: auto;
}
.screen--cmd { margin: 1.25rem 0 0; font-size: 0.9rem; }
@media (max-width: 560px) {
  .flow__item { grid-template-columns: 1fr; }
}
.cloud-hint { margin-top: 0.5rem; color: var(--ink-soft); }
</style>
