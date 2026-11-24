<template>
  <BiaStory ns="provisioner" :story="story" :graph="graph" :written="provisionerWritten" />
</template>

<script setup lang="ts">
import type { T } from '~/data/features'
import { buildProvisionerGraph, buildProvisionerStory, provisionerWritten } from '~/data/provisioner'

const t = useT()
const story = buildProvisionerStory(t as unknown as T)
const graph = buildProvisionerGraph(t as unknown as T)

useSeoMeta({ title: t('provisioner:seo_title'), description: t('provisioner:seo_description') })
</script>
