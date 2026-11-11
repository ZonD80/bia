<template>
  <div class="layout">
    <a class="skip" href="#content">Skip to content</a>
    <div class="stick">
      <CloudStrip v-if="route.path.replace(/\/$/, '') !== '/cloud'" />
      <SiteHeader />
    </div>
    <div id="content">
      <slot />
    </div>
    <SiteFooter />
  </div>
</template>

<script setup lang="ts">
const route = useRoute()
</script>

<style scoped>
/* the cloud strip and the header stick together */
.stick {
  position: sticky;
  top: 0;
  z-index: 30;
}
.stick :deep(.top) {
  position: relative;
}
.skip {
  position: absolute;
  left: -999px;
  top: 0;
  z-index: 100;
  padding: 0.6rem 1rem;
  background: var(--accent);
  color: #fff;
}
.skip:focus {
  left: 1rem;
  top: 1rem;
}
</style>
