<template>
  <figure class="film">
    <div class="film__frame">
      <iframe
        v-if="playing"
        :src="`https://www.youtube-nocookie.com/embed/${FRED_FILM.id}?autoplay=1&rel=0`"
        :title="FRED_FILM.title"
        allow="accelerometer; autoplay; clipboard-write; encrypted-media; gyroscope; picture-in-picture; web-share"
        referrerpolicy="strict-origin-when-cross-origin"
        allowfullscreen
      />
      <!-- YouTube loads only after a click: no third-party requests just for opening the page. -->
      <button v-else type="button" class="film__poster" :aria-label="`Play: ${FRED_FILM.title}`" @click="playing = true">
        <img :src="`https://i.ytimg.com/vi/${FRED_FILM.id}/hqdefault.jpg`" alt="" loading="lazy" />
        <span class="film__play" aria-hidden="true">
          <svg viewBox="0 0 24 24" width="30" height="30"><path d="M8 5.5v13l11-6.5z" fill="currentColor" /></svg>
        </span>
      </button>
    </div>
    <figcaption>
      Unedited. Fred made this promo film unaided, for about $15 of OpenRouter credit.
      <a :href="FRED_FILM.url" target="_blank" rel="noopener">Watch on YouTube</a>
    </figcaption>
  </figure>
</template>

<script setup lang="ts">
import { FRED_FILM } from '~/data/fred'

const playing = ref(false)
</script>

<style scoped>
.film {
  margin: 0;
}
.film__frame {
  position: relative;
  aspect-ratio: 16 / 9;
  border-radius: var(--radius);
  overflow: hidden;
  background: #1d2126;
  box-shadow: var(--shadow);
}
.film__frame iframe,
.film__poster {
  position: absolute;
  inset: 0;
  width: 100%;
  height: 100%;
  border: 0;
}
.film__poster {
  padding: 0;
  cursor: pointer;
  background: #1d2126;
}
.film__poster img {
  width: 100%;
  height: 100%;
  object-fit: cover;
  opacity: 0.85;
  transition: opacity 0.2s ease;
}
.film__poster:hover img {
  opacity: 1;
}
.film__play {
  position: absolute;
  top: 50%;
  left: 50%;
  display: grid;
  place-items: center;
  width: 4.5rem;
  height: 4.5rem;
  margin: -2.25rem 0 0 -2.25rem;
  border-radius: 50%;
  background: var(--accent);
  color: #fff;
  box-shadow: 0 8px 24px rgba(0, 0, 0, 0.3);
  transition: transform 0.2s ease;
}
.film__poster:hover .film__play {
  transform: scale(1.08);
}
.film figcaption {
  margin-top: 0.75rem;
  font-size: 0.92rem;
  color: var(--mute);
}
.film figcaption a {
  color: var(--accent);
  font-weight: 700;
}
</style>
