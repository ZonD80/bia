<template>
  <BiaStory ns="bobby" :story="story" :graph="graph" :written="bobbyWritten" no-intro />
</template>

<script setup lang="ts">
import type { T } from '~/data/features'
import { buildBobbyGraph, buildBobbyStory, bobbyWritten } from '~/data/bobby'

const t = useT()
const story = buildBobbyStory(t as unknown as T)
const graph = buildBobbyGraph(t as unknown as T)

useSeoMeta({ title: t('bobby:seo_title'), description: t('bobby:seo_description') })
</script>
