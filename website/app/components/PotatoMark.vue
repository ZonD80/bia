<template>
  <!-- generated from brand/potato.svg (tools/make-icons.sh makes the icons from the same file) -->
  <svg
    class="potato"
    viewBox="0 0 240 240"
    xmlns="http://www.w3.org/2000/svg"
    role="img"
    :aria-label="label"
  >
    <defs>
      <radialGradient :id="`${u}-skin`" cx="38%" cy="30%" r="82%">
        <stop offset="0%" stop-color="#e9cf98" />
        <stop offset="32%" stop-color="#d2ab6c" />
        <stop offset="66%" stop-color="#b3854a" />
        <stop offset="90%" stop-color="#8a5f2f" />
        <stop offset="100%" stop-color="#6c4722" />
      </radialGradient>
      <radialGradient :id="`${u}-shade`" cx="74%" cy="80%" r="62%">
        <stop offset="0%" stop-color="#2e1c0b" stop-opacity="0.6" />
        <stop offset="55%" stop-color="#2e1c0b" stop-opacity="0.2" />
        <stop offset="100%" stop-color="#2e1c0b" stop-opacity="0" />
      </radialGradient>
      <radialGradient :id="`${u}-gloss`" cx="30%" cy="24%" r="34%">
        <stop offset="0%" stop-color="#fff3d6" stop-opacity="0.6" />
        <stop offset="100%" stop-color="#fff3d6" stop-opacity="0" />
      </radialGradient>
      <radialGradient :id="`${u}-ground`" cx="50%" cy="50%" r="50%">
        <stop offset="0%" stop-color="#10151c" stop-opacity="0.38" />
        <stop offset="100%" stop-color="#10151c" stop-opacity="0" />
      </radialGradient>
      <radialGradient :id="`${u}-pit`" cx="45%" cy="40%" r="60%">
        <stop offset="0%" stop-color="#2f1c0a" />
        <stop offset="100%" stop-color="#6b4622" stop-opacity="0" />
      </radialGradient>
      <!-- optic: brushed metal housing, dark groove, amber lens -->
      <linearGradient :id="`${u}-metal`" x1="0" y1="0" x2="1" y2="1">
        <stop offset="0%" stop-color="#e6e9ec" />
        <stop offset="40%" stop-color="#9aa1a8" />
        <stop offset="70%" stop-color="#5d646b" />
        <stop offset="100%" stop-color="#2c3136" />
      </linearGradient>
      <linearGradient :id="`${u}-metal-in`" x1="1" y1="1" x2="0" y2="0">
        <stop offset="0%" stop-color="#d7dbdf" />
        <stop offset="55%" stop-color="#70777e" />
        <stop offset="100%" stop-color="#2a2f34" />
      </linearGradient>
      <radialGradient :id="`${u}-lens`" cx="50%" cy="50%" r="50%">
        <stop offset="0%" stop-color="#fffbe0" />
        <stop offset="22%" stop-color="#ffe680" />
        <stop offset="55%" stop-color="#ffbf1f" />
        <stop offset="85%" stop-color="#e07a00" />
        <stop offset="100%" stop-color="#7a3a00" />
      </radialGradient>
      <radialGradient :id="`${u}-halo`" cx="50%" cy="50%" r="50%">
        <stop offset="0%" stop-color="#ffcf3a" stop-opacity="0.7" />
        <stop offset="45%" stop-color="#ffb000" stop-opacity="0.3" />
        <stop offset="100%" stop-color="#ff9a00" stop-opacity="0" />
      </radialGradient>
      <linearGradient :id="`${u}-prong`" x1="0" y1="0" x2="1" y2="0">
        <stop offset="0%" stop-color="#6d747b" />
        <stop offset="45%" stop-color="#eef1f3" />
        <stop offset="100%" stop-color="#7c838a" />
      </linearGradient>
      <linearGradient :id="`${u}-copper`" x1="0" y1="0" x2="1" y2="0">
        <stop offset="0%" stop-color="#8a4a22" />
        <stop offset="45%" stop-color="#f0a46a" />
        <stop offset="100%" stop-color="#9a5226" />
      </linearGradient>
      <filter :id="`${u}-texture`" x="0" y="0" width="100%" height="100%">
        <feTurbulence type="fractalNoise" baseFrequency="1.5" numOctaves="2" seed="7" result="noise" />
        <feDiffuseLighting in="noise" surfaceScale="0.8" diffuseConstant="1.1" lighting-color="#ffffff" result="lit">
          <feDistantLight azimuth="225" elevation="58" />
        </feDiffuseLighting>
        <feComposite in="lit" in2="SourceGraphic" operator="in" result="clipped" />
        <feBlend in="SourceGraphic" in2="clipped" mode="multiply" result="fine" />
        <feComposite in="fine" in2="SourceGraphic" operator="arithmetic" k1="0" k2="0.5" k3="0.5" k4="0" />
      </filter>
      <filter :id="`${u}-lumps`" x="0" y="0" width="100%" height="100%">
        <feTurbulence type="fractalNoise" baseFrequency="0.03" numOctaves="3" seed="11" result="n" />
        <feColorMatrix in="n" type="matrix" values="0 0 0 0 0.38  0 0 0 0 0.24  0 0 0 0 0.1  0 0 0 1 -0.3" result="tint" />
        <feComposite in="tint" in2="SourceGraphic" operator="in" />
      </filter>
      <filter :id="`${u}-soft`" x="-50%" y="-50%" width="200%" height="200%">
        <feGaussianBlur stdDeviation="2.2" />
      </filter>
      <clipPath :id="`${u}-clip`">
        <path :id="`${u}-body`" d="M88 64c21-7 40-1 58-1 21-1 40-5 56 7 15 11 22 30 19 50-3 25-17 44-37 54-16 8-32 6-48 9-17 3-33 9-53 5-33-6-45-32-44-62 1-30 18-54 49-62z" />
      </clipPath>
    </defs>

    <!-- contact shadow -->
    <ellipse cx="126" cy="192" rx="94" ry="11" :fill="`url(#${u}-ground)`" />

    <!-- electrodes behind the skin line: only the part above the potato shows -->
    <g>
      <!-- left: zinc spike -->
      <g transform="rotate(-18 96 66)">
        <rect x="92" y="34" width="8" height="40" rx="1.5" :fill="`url(#${u}-prong)`" stroke="#3a3f44" stroke-width="0.8" />
        <rect x="90" y="30" width="12" height="10" rx="2" fill="#40464c" />
        <rect x="91" y="31" width="10" height="3" rx="1" fill="#8b9299" />
      </g>
      <!-- right: copper spike -->
      <g transform="rotate(16 172 70)">
        <rect x="168" y="38" width="8" height="40" rx="1.5" :fill="`url(#${u}-copper)`" stroke="#4a2810" stroke-width="0.8" />
        <rect x="166" y="34" width="12" height="10" rx="2" fill="#40464c" />
        <rect x="167" y="35" width="10" height="3" rx="1" fill="#8b9299" />
      </g>
    </g>

    <!-- body -->
    <g :clip-path="`url(#${u}-clip)`">
      <use :href="`#${u}-body`" :fill="`url(#${u}-skin)`" :filter="`url(#${u}-texture)`" />
      <use :href="`#${u}-body`" fill="#000" :filter="`url(#${u}-lumps)`" opacity="0.85" />
      <use :href="`#${u}-body`" :fill="`url(#${u}-shade)`" />
      <ellipse cx="94" cy="86" rx="60" ry="28" :fill="`url(#${u}-gloss)`" />
      <!-- potato eyes -->
      <g :fill="`url(#${u}-pit)`" opacity="0.6">
        <ellipse cx="72" cy="104" rx="5" ry="3.2" transform="rotate(20 72 104)" />
        <ellipse cx="190" cy="112" rx="4.6" ry="3" transform="rotate(-25 190 112)" />
        <ellipse cx="88" cy="160" rx="4.4" ry="2.8" transform="rotate(10 88 160)" />
        <ellipse cx="170" cy="160" rx="4" ry="2.6" transform="rotate(-12 170 160)" />
        <ellipse cx="142" cy="78" rx="3.8" ry="2.4" transform="rotate(-8 142 78)" />
      </g>
      <g fill="#2f1c0a" opacity="0.8">
        <ellipse cx="72" cy="104" rx="1.4" ry="1" />
        <ellipse cx="190" cy="112" rx="1.3" ry="0.9" />
        <ellipse cx="88" cy="160" rx="1.2" ry="0.9" />
        <ellipse cx="170" cy="160" rx="1.1" ry="0.8" />
      </g>
      <!-- specks and a little soil -->
      <g fill="#f2dcae" opacity="0.5">
        <circle cx="104" cy="74" r="1.1" /><circle cx="160" cy="92" r="1" /><circle cx="64" cy="128" r="0.9" />
        <circle cx="198" cy="138" r="0.9" /><circle cx="122" cy="168" r="1" /><circle cx="150" cy="148" r="0.8" />
      </g>
      <g fill="#4d3115" opacity="0.45">
        <circle cx="80" cy="84" r="0.8" /><circle cx="184" cy="92" r="0.9" /><circle cx="104" cy="146" r="0.8" />
        <circle cx="206" cy="124" r="0.7" /><circle cx="60" cy="150" r="0.8" />
      </g>
      <ellipse cx="176" cy="170" rx="10" ry="4" transform="rotate(-18 176 170)" fill="#4d3115" opacity="0.18" />
      <!-- puncture shadows where the electrodes go in -->
      <ellipse cx="95" cy="64" rx="6.5" ry="2.8" fill="#2a1808" opacity="0.6" />
      <ellipse cx="174.5" cy="63.5" rx="6.5" ry="2.8" fill="#2a1808" opacity="0.6" />
      <!-- shadow the optic casts on the skin -->
      <ellipse cx="132" cy="134" rx="32" ry="28" fill="#1e1206" opacity="0.35" :filter="`url(#${u}-soft)`" />
    </g>
    <use :href="`#${u}-body`" fill="none" stroke="#4a2f12" stroke-opacity="0.45" stroke-width="1.6" />

    <!-- wires: red from the zinc spike, black from the copper one, into the optic -->
    <g fill="none" stroke-linecap="round">
      <path d="M90 32c-22 4-32 26-26 48 4 18 14 30 28 36" stroke="#5a0f0f" stroke-width="5.4" />
      <path d="M90 32c-22 4-32 26-26 48 4 18 14 30 28 36" stroke="#d3302b" stroke-width="3.6" />
      <path d="M88 33c-18 5-27 24-22 44" stroke="#ff8f86" stroke-width="1" opacity="0.7" />
      <path d="M178 37c24 2 36 22 30 44-6 18-26 30-46 35" stroke="#050607" stroke-width="5.4" />
      <path d="M178 37c24 2 36 22 30 44-6 18-26 30-46 35" stroke="#24282d" stroke-width="3.6" />
      <path d="M180 38c20 3 30 20 27 38" stroke="#7d858d" stroke-width="1" opacity="0.7" />
    </g>

    <!-- optic -->
    <circle cx="126" cy="120" r="44" :fill="`url(#${u}-halo)`" class="potato__halo" />
    <g>
      <!-- bracket screwed into the potato -->
      <path d="M98 112h-6a4 4 0 0 0 0 8h6zM154 112h6a4 4 0 0 1 0 8h-6z" fill="#4b5157" />
      <circle cx="126" cy="117" r="27.5" fill="#15181b" />
      <circle cx="126" cy="116" r="26" :fill="`url(#${u}-metal)`" />
      <circle cx="126" cy="116" r="20.5" :fill="`url(#${u}-metal-in)`" />
      <circle cx="126" cy="116" r="17.2" fill="#0b0d0f" />
      <!-- four screws -->
      <g fill="#2a2f34">
        <circle cx="126" cy="93.5" r="1.7" /><circle cx="148.5" cy="116" r="1.7" />
        <circle cx="126" cy="138.5" r="1.7" /><circle cx="103.5" cy="116" r="1.7" />
      </g>
      <!-- lens -->
      <circle cx="126" cy="116" r="14.5" :fill="`url(#${u}-lens)`" class="potato__lens" />
      <circle cx="126" cy="116" r="5.2" fill="#fffdf0" opacity="0.92" />
      <ellipse cx="120" cy="109.5" rx="5" ry="3" transform="rotate(-35 120 109.5)" fill="#ffffff" opacity="0.75" />
      <path d="M113 124a15 15 0 0 0 23 3" fill="none" stroke="#fff4c4" stroke-width="1" opacity="0.45" />
    </g>
  </svg>
</template>

<script setup lang="ts">
withDefaults(defineProps<{ label?: string }>(), {
  label: 'bia — a potato wired to a glowing eye: anything can become someone',
})

// unique ids so several potatoes on one page do not share gradients/filters
const u = `p${useId().replace(/[^a-zA-Z0-9]/g, '')}`
</script>

<style scoped>
.potato {
  width: 100%;
  height: auto;
  display: block;
  overflow: visible;
}
/* the optic breathes, like it is thinking */
.potato__halo {
  transform-origin: 126px 120px;
  animation: glow 4.5s ease-in-out infinite;
}
.potato__lens {
  animation: lens 4.5s ease-in-out infinite;
}
@keyframes glow {
  0%, 100% { opacity: 0.75; transform: scale(0.94); }
  50% { opacity: 1; transform: scale(1.06); }
}
@keyframes lens {
  0%, 100% { filter: brightness(0.96); }
  50% { filter: brightness(1.12); }
}
@media (prefers-reduced-motion: reduce) {
  .potato__halo, .potato__lens { animation: none; }
}
</style>
