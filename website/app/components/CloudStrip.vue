<template>
  <div v-if="!closed" class="cloud-strip" role="region" aria-label="bia cloud">
    <div class="container cloud-strip__inner">
      <p class="cloud-strip__text">
        <span class="cloud-strip__tag">New</span>
        <span><strong>bia cloud</strong> — no computer to spare? Wake up a bia on its own server, run for you.</span>
        <a class="cloud-strip__link" href="https://cloud.ownbia.com/">Try bia cloud →</a>
      </p>
      <button type="button" class="cloud-strip__x" aria-label="Close" @click="close">
        <svg viewBox="0 0 24 24" width="18" height="18" aria-hidden="true">
          <path d="M6 6l12 12M18 6L6 18" stroke="currentColor" stroke-width="2.2" stroke-linecap="round" />
        </svg>
      </button>
    </div>
  </div>
</template>

<script setup lang="ts">
const KEY = 'bia.cloudStrip.closed'
const closed = ref(false)

function close() {
  closed.value = true
  try {
    localStorage.setItem(KEY, '1')
  } catch {}
  document.documentElement.classList.add('no-cloud-strip')
}

// Prerendered pages: hide it before the first paint when it was closed before (no flash).
useHead({
  script: [
    {
      key: 'cloud-strip',
      innerHTML: `try{if(localStorage.getItem('${KEY}'))document.documentElement.classList.add('no-cloud-strip')}catch(e){}`,
    },
  ],
})

onMounted(() => {
  try {
    closed.value = !!localStorage.getItem(KEY)
  } catch {}
})
</script>

<style>
/* its height, for anchors that land below the sticky header */
:root {
  --strip-h: 2.6rem;
}
@media (max-width: 560px) {
  :root {
    --strip-h: 3.6rem;
  }
}
:root.no-cloud-strip {
  --strip-h: 0px;
}
.no-cloud-strip .cloud-strip {
  display: none;
}
</style>

<style scoped>
.cloud-strip {
  background: var(--accent-deep);
  color: #fff;
  font-size: 0.92rem;
}
.cloud-strip__inner {
  display: flex;
  align-items: center;
  gap: 0.75rem;
  min-height: 2.6rem;
  padding-top: 0.35rem;
  padding-bottom: 0.35rem;
}
.cloud-strip__text {
  flex: 1;
  margin: 0;
  display: flex;
  flex-wrap: wrap;
  align-items: center;
  justify-content: center;
  gap: 0.35rem 0.7rem;
  text-align: center;
  line-height: 1.35;
}
.cloud-strip__tag {
  background: var(--tan);
  color: var(--ink);
  font-weight: 800;
  font-size: 0.72rem;
  letter-spacing: 0.06em;
  text-transform: uppercase;
  padding: 0.12rem 0.5rem;
  border-radius: 999px;
}
.cloud-strip__link {
  color: #fff;
  font-weight: 700;
  text-decoration: underline;
  text-underline-offset: 3px;
  white-space: nowrap;
}
.cloud-strip__link:hover {
  color: var(--tan-soft);
}
.cloud-strip__x {
  flex: none;
  display: grid;
  place-items: center;
  width: 2rem;
  height: 2rem;
  border: 0;
  border-radius: 50%;
  background: transparent;
  color: #fff;
  cursor: pointer;
  opacity: 0.85;
}
.cloud-strip__x:hover,
.cloud-strip__x:focus-visible {
  opacity: 1;
  background: rgba(255, 255, 255, 0.14);
}
@media (max-width: 560px) {
  .cloud-strip {
    font-size: 0.85rem;
  }
  .cloud-strip__text {
    justify-content: flex-start;
    text-align: left;
  }
}
</style>
