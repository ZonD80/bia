<template>
  <BiaStory ns="jessica" :story="story" :graph="graph" :written="jessicaWritten" />
</template>

<script setup lang="ts">
import type { T } from '~/data/features'
import { buildJessicaGraph, buildJessicaStory, jessicaWritten } from '~/data/jessica'

const t = useT()
const story = buildJessicaStory(t as unknown as T)
const graph = buildJessicaGraph(t as unknown as T)

useSeoMeta({ title: t('jessica:seo_title'), description: t('jessica:seo_description') })
</script>
