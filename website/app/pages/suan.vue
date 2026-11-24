<template>
  <BiaStory ns="suan" :story="story" :graph="graph" :written="suanWritten" />
</template>

<script setup lang="ts">
import type { T } from '~/data/features'
import { buildSuanGraph, buildSuanStory, suanWritten } from '~/data/suan'

const t = useT()
const story = buildSuanStory(t as unknown as T)
const graph = buildSuanGraph(t as unknown as T)

useSeoMeta({ title: t('suan:seo_title'), description: t('suan:seo_description') })
</script>
