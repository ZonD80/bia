<template>
  <span class="ico" :style="{ width: size, height: size }" aria-hidden="true">
    <PotatoMark v-if="name === 'potato'" />
    <svg v-else viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.7" stroke-linecap="round" stroke-linejoin="round" v-html="paths[name] || paths.sparkle" />
  </span>
</template>

<script setup lang="ts">
withDefaults(defineProps<{ name: string; size?: string }>(), { size: '1.5rem' })

const paths: Record<string, string> = {
  chat: '<path d="M4 5h16v11H9l-5 4z"/><path d="M8 9h8M8 12h5"/>',
  phone: '<path d="M6.5 3h3l1.5 4.5-2 1.3a11 11 0 0 0 6.2 6.2l1.3-2 4.5 1.5v3a2 2 0 0 1-2.2 2A17 17 0 0 1 4.5 5.2 2 2 0 0 1 6.5 3z"/>',
  globe: '<circle cx="12" cy="12" r="9"/><path d="M3 12h18M12 3c2.8 3 2.8 15 0 18M12 3c-2.8 3-2.8 15 0 18"/>',
  window: '<rect x="3" y="4" width="18" height="16" rx="2"/><path d="M3 9h18M7 6.5h.01M10 6.5h.01"/>',
  sparkle: '<path d="M12 3l1.8 5.2L19 10l-5.2 1.8L12 17l-1.8-5.2L5 10l5.2-1.8z"/><path d="M19 16l.8 2.2L22 19l-2.2.8L19 22l-.8-2.2L16 19l2.2-.8z"/>',
  people: '<circle cx="9" cy="8" r="3.2"/><path d="M3 20c.5-3.5 3-5.5 6-5.5s5.5 2 6 5.5"/><circle cx="17" cy="9" r="2.5"/><path d="M16 14.5c2.6.2 4.5 2 5 4.5"/>',
  book: '<path d="M4 5a2 2 0 0 1 2-2h13v16H6a2 2 0 0 0-2 2z"/><path d="M4 19V5M8 7h7M8 10.5h7"/>',
  clock: '<circle cx="12" cy="12" r="9"/><path d="M12 7v5l3 2"/>',
  brain: '<path d="M9 4a3 3 0 0 0-3 3 3 3 0 0 0-2 5 3 3 0 0 0 2 5 3 3 0 0 0 3 3h1V4z"/><path d="M15 4a3 3 0 0 1 3 3 3 3 0 0 1 2 5 3 3 0 0 1-2 5 3 3 0 0 1-3 3h-1V4z"/>',
  terminal: '<rect x="3" y="4" width="18" height="16" rx="2"/><path d="M7 9l3 3-3 3M12 15h5"/>',
  browser: '<circle cx="12" cy="12" r="9"/><circle cx="12" cy="12" r="3.5"/><path d="M12 8.5h8.5M8.9 13.8L4.6 6.4M15 13.8l-4.3 7.1"/>',
  cursor: '<path d="M5 3l14 7-6 2-2 6z"/><path d="M13 12l5 5"/>',
  key: '<circle cx="8" cy="15" r="4"/><path d="M11 12l8-8M16 7l2 2M14 9l2 2"/>',
  hand: '<path d="M8 13V5.5a1.5 1.5 0 0 1 3 0V11M11 10V4.5a1.5 1.5 0 0 1 3 0V11M14 10.5V6a1.5 1.5 0 0 1 3 0v7c0 4-2.5 7-6.5 7-3 0-4.5-1.5-6-4l-2-3.5a1.5 1.5 0 0 1 2.6-1.5L8 13"/>',
  stick: '<rect x="7" y="9" width="10" height="12" rx="2"/><path d="M9 9V3h6v6M11 5v1.5M13 5v1.5"/>',
  move: '<rect x="2" y="6" width="8" height="10" rx="1.5"/><rect x="14" y="6" width="8" height="10" rx="1.5"/><path d="M8 19c2 2 6 2 8 0M14 17l2 2-2 2"/>',
  refresh: '<path d="M20 12a8 8 0 0 1-14 5.3M4 12a8 8 0 0 1 14-5.3"/><path d="M18 3v4h-4M6 21v-4h4"/>',
  link: '<path d="M10 14a4 4 0 0 0 5.7 0l3-3a4 4 0 0 0-5.7-5.7l-1 1"/><path d="M14 10a4 4 0 0 0-5.7 0l-3 3a4 4 0 0 0 5.7 5.7l1-1"/>',
  puzzle: '<path d="M9 3h4v3a2 2 0 1 0 4 0V3h4v6h-3a2 2 0 1 0 0 4h3v8h-6v-3a2 2 0 1 0-4 0v3H3v-8h3a2 2 0 1 0 0-4H3V3z"/>',
  shield: '<path d="M12 3l8 3v6c0 4.5-3.4 8.2-8 9-4.6-.8-8-4.5-8-9V6z"/><path d="M8.5 12l2.5 2.5L15.5 10"/>',
  home: '<path d="M3 11l9-7 9 7"/><path d="M5 10v10h14V10M10 20v-5h4v5"/>',
  shop: '<path d="M4 9l1.5-5h13L20 9M4 9h16v11H4z"/><path d="M4 9a3 3 0 0 0 5.3 1.9A3 3 0 0 0 14.7 11 3 3 0 0 0 20 9M10 20v-5h4v5"/>',
  plane: '<path d="M21 15l-8-4V5a1.5 1.5 0 0 0-3 0v6l-8 4v2l8-2v4l-2 1.5V22l3.5-1L15 22v-1.5L13 19v-4l8 2z"/>',
  briefcase: '<rect x="3" y="7" width="18" height="13" rx="2"/><path d="M9 7V5a2 2 0 0 1 2-2h2a2 2 0 0 1 2 2v2M3 12h18"/>',
  school: '<path d="M2 9l10-5 10 5-10 5z"/><path d="M6 11v5c3 2.5 9 2.5 12 0v-5M22 9v5"/>',
  mountain: '<path d="M2 20l7-12 4 6 2-3 7 9z"/><path d="M8 10l1.5 1.5L11 10"/>',
  arrow: '<path d="M5 12h14M13 6l6 6-6 6"/>',
  check: '<path d="M5 12.5l4.5 4.5L19 7.5"/>',
  usb: '<path d="M12 3v14M12 3l-2 2.5M12 3l2 2.5M8 11v2l4 3 4-3v-3"/><circle cx="12" cy="19" r="2"/><rect x="15" y="8" width="2.5" height="2.5"/><circle cx="8" cy="9.5" r="1.3"/>',
  power: '<path d="M12 3v8"/><path d="M6.5 6.5a8 8 0 1 0 11 0"/>',
  wave: '<path d="M7 11.5V9a1.5 1.5 0 0 1 3 0v2M10 10V6.5a1.5 1.5 0 0 1 3 0V10M13 10V7.5a1.5 1.5 0 0 1 3 0V13c0 4-2.5 7-6 7-2.5 0-4-1.2-5.5-3.5L3 13.5a1.4 1.4 0 0 1 2.3-1.6L7 14"/><path d="M18 3c1.5 1 2.5 2.5 2.8 4.5"/>',
}
</script>

<style scoped>
.ico {
  display: inline-flex;
  flex-shrink: 0;
}
.ico > svg {
  width: 100%;
  height: 100%;
}
</style>
