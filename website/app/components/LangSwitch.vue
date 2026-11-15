<template>
  <details ref="box" class="lang">
    <summary :aria-label="$t('common:language')">
      <svg viewBox="0 0 24 24" width="18" height="18" fill="none" stroke="currentColor" stroke-width="1.8" aria-hidden="true">
        <circle cx="12" cy="12" r="9" /><path d="M3 12h18M12 3c2.8 3 2.8 15 0 18M12 3c-2.8 3-2.8 15 0 18" />
      </svg>
      <span>{{ locale.toUpperCase() }}</span>
    </summary>
    <div class="lang__list">
      <NuxtLink
        v-for="l in LOCALES" :key="l" :to="to(l)" :hreflang="l" :lang="l"
        :aria-current="l === locale ? 'true' : undefined" @click="choose(l)"
      >{{ LOCALE_NAMES[l] }}</NuxtLink>
    </div>
  </details>
</template>

<script setup lang="ts">
// The language menu. Each entry is a real link to the same page in that language;
// choosing one is remembered, so the next visit opens in it (see DETECT_SCRIPT).
const locale = useLocale()
const route = useRoute()
const box = ref<HTMLDetailsElement>()

const to = (l: Locale) => localePath(stripLocale(route.fullPath), l)

// a tap anywhere else closes the list
function outside(e: Event) {
  if (box.value?.open && !box.value.contains(e.target as Node)) box.value.open = false
}
onMounted(() => document.addEventListener('click', outside))
onBeforeUnmount(() => document.removeEventListener('click', outside))

function choose(l: Locale) {
  try {
    localStorage.setItem(LOCALE_KEY, l)
  } catch {
    // private windows: the choice lasts for this visit
  }
  if (box.value) box.value.open = false
}
</script>

<style scoped>
.lang { position: relative; }
.lang summary {
  display: inline-flex;
  align-items: center;
  gap: 0.3rem;
  list-style: none;
  cursor: pointer;
  font-weight: 600;
  font-size: 0.9rem;
  color: var(--ink-soft);
  padding: 0.35rem 0.5rem;
  border: 1px solid var(--line);
  border-radius: 999px;
}
.lang summary::-webkit-details-marker { display: none; }
.lang summary:hover, .lang[open] summary { color: var(--accent); border-color: var(--accent); }
.lang__list {
  position: absolute;
  right: 0;
  top: calc(100% + 0.4rem);
  z-index: 40;
  display: grid;
  min-width: 9rem;
  padding: 0.35rem;
  background: var(--card, #fff);
  border: 1px solid var(--line);
  border-radius: 0.8rem;
  box-shadow: 0 12px 24px rgba(29, 33, 38, 0.12);
}
.lang__list a {
  padding: 0.5rem 0.7rem;
  border-radius: 0.5rem;
  color: var(--ink);
  text-decoration: none;
  font-weight: 600;
  font-size: 0.95rem;
}
.lang__list a:hover { background: var(--accent-soft); }
.lang__list a[aria-current] { color: var(--accent); }
</style>
