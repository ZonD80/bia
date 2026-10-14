<template>
  <div class="layout">
    <a class="skip" href="#content">Skip to content</a>
    <SiteHeader />
    <div id="content">
      <slot />
    </div>
    <SiteFooter />
  </div>
</template>

<style scoped>
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
