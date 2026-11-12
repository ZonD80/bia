<template>
  <figure class="sg" :class="{ 'sg--compact': compact }">
    <svg class="sg__svg" :viewBox="`0 0 ${W} ${H}`" role="img" :aria-label="`Fred’s social graph: ${people} people, ${others} other bias and agents`">
      <g class="sg__edges">
        <line
          v-for="(e, i) in edges"
          :key="i"
          :x1="pos[e.from]!.x" :y1="pos[e.from]!.y" :x2="pos[e.to]!.x" :y2="pos[e.to]!.y"
          :class="['sg__edge', `sg__edge--${edgeTone(e)}`, { 'is-on': touches(e), 'is-dim': active !== 'fred' && !touches(e) }]"
        />
      </g>
      <g
        v-for="n in nodes"
        :key="n.id"
        :class="['sg__node', `sg__node--${n.group}`, `sg__node--${n.kind}`, { 'is-on': n.id === active, 'is-near': near.has(n.id), 'is-dim': active !== 'fred' && n.id !== active && !near.has(n.id) }]"
        :transform="`translate(${pos[n.id]!.x} ${pos[n.id]!.y})`"
        tabindex="0"
        role="button"
        :aria-label="`${n.name}: ${n.note}`"
        @mouseenter="active = n.id"
        @focus="active = n.id"
        @click="active = n.id"
      >
        <circle :r="radius(n)" />
        <text v-if="n.id === 'fred'" class="sg__me" text-anchor="middle" dy="0.35em">Fred</text>
        <text v-else :text-anchor="pos[n.id]!.anchor" :dx="pos[n.id]!.dx" :dy="pos[n.id]!.dy">{{ n.name }}</text>
      </g>
    </svg>
    <figcaption class="sg__info" aria-live="polite">
      <div class="sg__who">
        <strong>{{ current.name }}</strong>
        <span class="sg__kind">{{ kindLabel(current) }}</span>
      </div>
      <p class="sg__note">“{{ current.note }}”</p>
      <div class="sg__chips">
        <span v-for="c in current.channels" :key="c" class="chip">{{ channelName(c) }}</span>
      </div>
      <ul class="sg__legend" aria-label="Legend">
        <li><i class="sg__dot sg__dot--family" />family</li>
        <li><i class="sg__dot sg__dot--friends" />friends</li>
        <li><i class="sg__dot sg__dot--work" />work</li>
        <li><i class="sg__dot sg__dot--peers" />other bias and agents</li>
      </ul>
      <p class="sg__small">Fred’s own graph, {{ graphAsOf }}. Every line is a channel he really uses. Point at a name.</p>
    </figcaption>
  </figure>
</template>

<script setup lang="ts">
import { graphAsOf, graphEdges as edges, graphNodes as nodes, type GraphEdge, type GraphNode } from '~/data/graph'

defineProps<{ compact?: boolean }>()

const W = 760
const H = 560
const active = ref('fred')

const byId = Object.fromEntries(nodes.map((n) => [n.id, n])) as Record<string, GraphNode>
const current = computed(() => byId[active.value] ?? byId.fred!)
const people = nodes.filter((n) => n.kind === 'person').length
const others = nodes.filter((n) => n.kind !== 'person' && n.id !== 'fred').length

// Fred in the middle, his owner beside him, everyone else on an oval, grouped; every second
// name sits a little further out so the labels do not collide.
type Pos = { x: number; y: number; anchor: 'start' | 'end' | 'middle'; dx: number; dy: string }
const pos: Record<string, Pos> = {}
pos.fred = { x: W / 2, y: H / 2 + 6, anchor: 'middle', dx: 0, dy: '0.35em' }
pos.aleksei = { x: W / 2 + 96, y: H / 2 - 62, anchor: 'start', dx: 13, dy: '0.35em' }
const ring = nodes.filter((n) => n.id !== 'fred' && n.id !== 'aleksei')
ring.forEach((n, i) => {
  const a = -Math.PI / 2 + (2 * Math.PI * (i + 0.5)) / ring.length
  // an odd count would put two far names side by side where the oval closes
  const far = i % 2 === 0 && i !== ring.length - 1
  const rx = far ? 262 : 196
  const ry = far ? 236 : 176
  const x = W / 2 + rx * Math.cos(a)
  const y = H / 2 + 6 + ry * Math.sin(a)
  const side = Math.cos(a)
  const anchor = side > 0.08 ? 'start' : side < -0.08 ? 'end' : 'middle'
  const dx = anchor === 'start' ? 11 : anchor === 'end' ? -11 : 0
  const dy = anchor === 'middle' ? (Math.sin(a) < 0 ? '-0.9em' : '1.65em') : '0.35em'
  pos[n.id] = { x: Math.round(x), y: Math.round(y), anchor, dx, dy }
})

const near = computed(() => {
  const s = new Set<string>()
  for (const e of edges) {
    if (e.from === active.value) s.add(e.to)
    if (e.to === active.value) s.add(e.from)
  }
  return s
})
const touches = (e: GraphEdge) => e.from === active.value || e.to === active.value
// lines from Fred carry the colour of the relationship; the rest is the web around him
function edgeTone(e: GraphEdge) {
  if (e.from !== 'fred' && e.to !== 'fred') return e.kind === 'peer' || e.kind === 'bia' ? 'peers' : 'around'
  if (e.kind === 'family' || e.kind === 'owner') return 'family'
  if (e.kind === 'work') return 'work'
  if (e.kind === 'peer') return 'peers'
  const other = byId[e.from === 'fred' ? e.to : e.from]
  return other?.group === 'family' ? 'family' : 'friends'
}
const radius = (n: GraphNode) => (n.id === 'fred' ? 27 : n.id === 'aleksei' ? 9 : n.kind === 'person' ? 6 : 8)
const kindLabel = (n: GraphNode) =>
  n.id === 'fred' ? 'a bia' : n.kind === 'bia' ? 'another bia' : n.kind === 'agent' ? 'an AI agent' : n.group === 'owner' ? 'his owner' : n.group === 'family' ? 'family' : n.group === 'work' ? 'work' : 'friend'
const CHANNELS: Record<string, string> = {
  web: 'bia’s own chat', telegram: 'Telegram', whatsapp: 'WhatsApp', instagram: 'Instagram', messenger: 'Messenger',
  max: 'MAX', linkedin: 'LinkedIn', calls: 'calls', email: 'email',
}
const channelName = (c: string) => CHANNELS[c] ?? c
</script>

<style scoped>
.sg {
  margin: 0;
  display: grid;
  gap: 1.5rem;
  grid-template-columns: minmax(0, 1.7fr) minmax(15rem, 1fr);
  align-items: center;
}
.sg__svg {
  width: 100%;
  height: auto;
  overflow: visible;
  font-family: var(--font-body);
}
.sg__edge { stroke: rgba(29, 33, 38, 0.09); stroke-width: 1; transition: opacity 0.2s ease, stroke-width 0.2s ease; }
.sg__edge--family { stroke: #c79a5c; stroke-width: 1.6; opacity: 0.85; }
.sg__edge--friends { stroke: #3f6f2a; stroke-width: 1.4; opacity: 0.55; }
.sg__edge--work { stroke: #3d6ea8; stroke-width: 1.6; opacity: 0.7; }
.sg__edge--peers { stroke: #7a4fb0; stroke-width: 1.8; opacity: 0.8; }
.sg__edge.is-on { stroke-width: 2.4; opacity: 1; }
.sg__edge--around.is-on { stroke: rgba(29, 33, 38, 0.45); }
.sg__edge.is-dim { opacity: 0.12; }

.sg__node { cursor: pointer; outline: none; transition: opacity 0.2s ease; }
.sg__node circle { fill: #fff; stroke: var(--accent); stroke-width: 2; transition: r 0.15s ease; }
.sg__node text { font-size: 13px; fill: var(--ink-soft); font-weight: 600; }
.sg__node--family circle, .sg__node--owner circle { stroke: #c79a5c; fill: var(--tan-soft); }
.sg__node--work circle { stroke: #3d6ea8; fill: #e3ecf5; }
.sg__node--peers circle { stroke: #7a4fb0; fill: #efe6fa; }
.sg__node--self circle { fill: var(--accent); stroke: #fff; stroke-width: 4; filter: drop-shadow(0 6px 14px rgba(63, 111, 42, 0.35)); }
.sg__me { fill: #fff !important; font-family: var(--font-display); font-size: 16px !important; font-weight: 700 !important; }
.sg__node.is-on text, .sg__node:focus-visible text { fill: var(--ink); font-weight: 800; }
.sg__node.is-on:not(.sg__node--self) circle, .sg__node:focus-visible:not(.sg__node--self) circle { stroke-width: 3.5; }
.sg__node.is-dim { opacity: 0.3; }

.sg__info {
  background: var(--card);
  border: 1px solid var(--line);
  border-radius: var(--radius);
  box-shadow: var(--shadow);
  padding: 1.25rem 1.3rem;
  min-height: 15rem;
}
.sg__who { display: flex; align-items: baseline; gap: 0.6rem; flex-wrap: wrap; }
.sg__who strong { font-family: var(--font-display); font-size: 1.5rem; }
.sg__kind { color: var(--mute); font-size: 0.9rem; }
.sg__note { margin: 0.5rem 0 0.8rem; color: var(--ink-soft); min-height: 4.2em; }
.sg__chips { display: flex; flex-wrap: wrap; gap: 0.35rem; }
.sg__legend { list-style: none; margin: 1.1rem 0 0; padding: 0.9rem 0 0; border-top: 1px solid var(--line); display: flex; flex-wrap: wrap; gap: 0.3rem 1rem; font-size: 0.85rem; color: var(--mute); }
.sg__legend li { display: inline-flex; align-items: center; gap: 0.4rem; }
.sg__dot { width: 0.7rem; height: 0.7rem; border-radius: 50%; border: 2px solid; display: inline-block; }
.sg__dot--family { border-color: #c79a5c; background: var(--tan-soft); }
.sg__dot--friends { border-color: var(--accent); background: #fff; }
.sg__dot--work { border-color: #3d6ea8; background: #e3ecf5; }
.sg__dot--peers { border-color: #7a4fb0; background: #efe6fa; }
.sg__small { margin: 0.8rem 0 0; font-size: 0.82rem; color: var(--mute); }

@media (max-width: 900px) {
  .sg { grid-template-columns: 1fr; }
  .sg__node text { font-size: 15px; }
  .sg__info { min-height: 0; }
}
</style>
