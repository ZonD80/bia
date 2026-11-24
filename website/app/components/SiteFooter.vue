<template>
  <footer class="foot">
    <div class="container foot__grid">
      <div class="foot__brand">
        <div class="foot__row">
          <span class="foot__potato"><PotatoMark label="" /></span>
          <p class="foot__name">bia</p>
        </div>
        <p class="foot__motto">{{ $t('common:boot_into_agent_making_every_potato') }}</p>
      </div>
      <nav class="foot__col" :aria-label="$t('common:talks_with_people')">
        <h3>{{ $t('common:talks') }}</h3>
        <NuxtLink v-for="f in byGroup('talk')" :key="f.slug" :to="$lp(`/features/${f.slug}`)">{{ f.name }}</NuxtLink>
      </nav>
      <nav class="foot__col" :aria-label="$t('common:thinks_and_remembers')">
        <h3>{{ $t('common:thinks') }}</h3>
        <NuxtLink v-for="f in byGroup('mind')" :key="f.slug" :to="$lp(`/features/${f.slug}`)">{{ f.name }}</NuxtLink>
      </nav>
      <nav class="foot__col" :aria-label="$t('common:gets_things_done')">
        <h3>{{ $t('common:does') }}</h3>
        <NuxtLink v-for="f in byGroup('hands')" :key="f.slug" :to="$lp(`/features/${f.slug}`)">{{ f.name }}</NuxtLink>
      </nav>
      <nav class="foot__col" :aria-label="$t('common:lives_and_grows')">
        <h3>{{ $t('common:lives') }}</h3>
        <NuxtLink v-for="f in byGroup('life')" :key="f.slug" :to="$lp(`/features/${f.slug}`)">{{ f.name }}</NuxtLink>
      </nav>
      <nav class="foot__col" :aria-label="$t('common:more')">
        <h3>{{ $t('common:more') }}</h3>
        <NuxtLink :to="$lp('/use-cases')">{{ $t('common:use_cases') }}</NuxtLink>
        <NuxtLink :to="$lp('/how-it-works')">{{ $t('common:how_it_works') }}</NuxtLink>
        <NuxtLink :to="$lp('/why-bia')">{{ $t('common:why_bia') }}</NuxtLink>
        <NuxtLink :to="$lp('/cases')">{{ $t('common:real_cases') }}</NuxtLink>
        <NuxtLink :to="$lp('/faq')">{{ $t('common:faq') }}</NuxtLink>
        <NuxtLink :to="$lp('/download')">{{ $t('common:get_bia') }}</NuxtLink>
        <NuxtLink :to="$lp('/servers')">{{ $t('common:servers_and_vms') }}</NuxtLink>
        <NuxtLink :to="$lp('/cloud')">{{ $t('common:bia_cloud_we_run_it_for') }}</NuxtLink>
        <NuxtLink :to="$lp('/privacy')">{{ $t('common:privacy') }}</NuxtLink>
        <a :href="REPO_URL" target="_blank" rel="noopener">{{ $t('common:source_code_on_github') }}</a>
      </nav>
    </div>
    <div class="container foot__base">
      <span><Tr k="common:bia_is_free_software_gpl_3" :v="{ REPO_URL: REPO_URL.replace('https://', '') }"><a :href="REPO_URL" target="_blank" rel="noopener" /></Tr></span>
    </div>
  </footer>
</template>

<script setup lang="ts">
import { featuresFor, type Feature } from '~/data/features'

// the footer stays while the language changes
const { $i18n } = useNuxtApp()
const locale = useLocale()
const features = computed(() => (locale.value, featuresFor($i18n).features))

const byGroup = (g: Feature['group']) => features.value.filter((f) => f.group === g)
</script>

<style scoped>
.foot {
  background: #1d2126;
  color: rgba(251, 248, 242, 0.72);
  padding-block: 3.5rem 2rem;
}
.foot__grid {
  display: grid;
  gap: 2rem;
  grid-template-columns: repeat(auto-fit, minmax(min(100%, 11rem), 1fr));
}
.foot__brand {
  grid-column: span 2;
  min-width: min(100%, 16rem);
}
.foot__row {
  display: flex;
  align-items: center;
  gap: 0.6rem;
}
.foot__potato {
  width: 3rem;
}
.foot__name {
  margin: 0;
  font-family: var(--font-display);
  font-weight: 700;
  font-size: 2rem;
  color: #fff;
}
.foot__motto {
  margin-top: 0.75rem;
  max-width: 22rem;
}
.foot__col {
  display: flex;
  flex-direction: column;
  gap: 0.45rem;
  font-size: 0.92rem;
}
.foot__col h3 {
  font-family: var(--font-body);
  font-size: 0.78rem;
  letter-spacing: 0.12em;
  text-transform: uppercase;
  color: #fff;
  margin-bottom: 0.4rem;
}
.foot__col a {
  color: rgba(251, 248, 242, 0.72);
  text-decoration: none;
}
.foot__col a:hover {
  color: #fff;
}
.foot__base a {
  color: #fff;
}
.foot__base {
  margin-top: 2.5rem;
  padding-top: 1.5rem;
  border-top: 1px solid rgba(255, 255, 255, 0.1);
  font-size: 0.86rem;
}
@media (max-width: 640px) {
  .foot__brand {
    grid-column: 1 / -1;
  }
}
</style>
