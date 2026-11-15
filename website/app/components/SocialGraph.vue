<template>
  <figure class="sg" :class="{ 'sg--compact': compact }">
    <div class="sg__scroll">
      <svg class="sg__svg" :viewBox="`0 0 ${W} ${H}`" role="img" :aria-label="$t('graph:aria', { people, others, clusters: circles.length - 1 })">
        <!-- the circles: one soft island per cluster, with its name -->
        <g
          v-for="c in islands"
          :key="c.id"
          :class="['sg__isle', `sg__isle--${c.id}`, { 'is-on': activeCircle === c.id, 'is-dim': dimCircle(c.id) }]"
          @mouseenter="pick(c.id, true)"
          @click="pick(c.id, true)"
        >
          <ellipse :cx="c.x" :cy="c.y" :rx="c.rx" :ry="c.ry" />
          <text class="sg__isle-name" :x="c.x" :y="c.y - c.ry - 9" text-anchor="middle">{{ c.title }}</text>
        </g>
        <g class="sg__edges">
          <line
            v-for="(e, i) in edges"
            :key="i"
            :x1="pos[e.from]!.x" :y1="pos[e.from]!.y" :x2="pos[e.to]!.x" :y2="pos[e.to]!.y"
            :class="['sg__edge', `sg__edge--${edgeTone(e)}`, { 'is-on': touches(e), 'is-dim': focused && !touches(e) }]"
          />
        </g>
        <g
          v-for="n in nodes"
          :key="n.id"
          :class="['sg__node', `sg__node--${n.cluster}`, `sg__node--${n.kind}`, { 'sg__node--me': n.id === 'fred', 'is-on': n.id === active, 'is-near': near.has(n.id), 'is-dim': dimNode(n) }]"
          :transform="`translate(${pos[n.id]!.x} ${pos[n.id]!.y})`"
          tabindex="0"
          role="button"
          :aria-label="`${n.name}: ${n.note}`"
          @mouseenter="pick(n.id)"
          @focus="pick(n.id)"
          @click="pick(n.id)"
        >
          <circle :r="radius(n)" />
          <text v-if="n.id === 'fred'" class="sg__me" text-anchor="middle" dy="0.35em">Fred</text>
          <text v-else :text-anchor="pos[n.id]!.anchor" :dx="pos[n.id]!.dx" :dy="pos[n.id]!.dy">{{ n.name }}</text>
        </g>
      </svg>
    </div>
    <figcaption class="sg__info" aria-live="polite">
      <template v-if="shownCircle">
        <div class="sg__who">
          <strong>{{ shownCircle.title }}</strong>
          <span class="sg__kind">{{ $t('graph:circle') }} · {{ $t('graph:people_count', { n: members(shownCircle.id) }) }}</span>
        </div>
        <p class="sg__note">“{{ shownCircle.note }}”</p>
      </template>
      <template v-else>
        <div class="sg__who">
          <strong>{{ current.name }}</strong>
          <span class="sg__kind">{{ kindLabel(current) }}</span>
        </div>
        <p class="sg__note">“{{ current.note }}”</p>
        <div class="sg__chips">
          <span v-for="c in current.channels" :key="c" class="chip">{{ channelName(c) }}</span>
        </div>
      </template>
      <ul class="sg__legend" :aria-label="$t('graph:legend')">
        <li v-for="c in circles.filter((x) => x.id !== 'self')" :key="c.id">
          <button type="button" :class="{ 'is-on': activeCircle === c.id }" @mouseenter="pick(c.id, true)" @focus="pick(c.id, true)" @click="pick(c.id, true)">
            <i :class="['sg__dot', `sg__dot--${c.id}`]" />{{ c.title }}
          </button>
        </li>
      </ul>
      <p class="sg__small">{{ $t('graph:small', { date: longDate(graphAsOf, locale) }) }}</p>
    </figcaption>
  </figure>
</template>

<script setup lang="ts">
import type { T } from '~/data/features'
import { buildGraphCircles, buildGraphNodes, graphAsOf, graphEdges as edges, type GraphCluster, type GraphEdge, type GraphNode } from '~/data/graph'

defineProps<{ compact?: boolean }>()

const t = useT() as unknown as T
const locale = useLocale()
const nodes = buildGraphNodes(t)
const circles = buildGraphCircles(t)

const W = 900
const H = 720
// what the panel shows: a node (Fred at first) or a whole circle
const active = ref('fred')
const activeCircle = ref<GraphCluster | ''>('')
function pick(id: string, circle = false) {
  if (circle) activeCircle.value = id as GraphCluster
  else {
    active.value = id
    activeCircle.value = ''
  }
}

const byId = Object.fromEntries(nodes.map((n) => [n.id, n])) as Record<string, GraphNode>
const current = computed(() => byId[active.value] ?? byId.fred!)
const shownCircle = computed(() => circles.find((c) => c.id === activeCircle.value))
const people = nodes.filter((n) => n.kind === 'person').length
const others = nodes.filter((n) => n.kind !== 'person' && n.id !== 'fred').length
const members = (c: GraphCluster) => nodes.filter((n) => n.cluster === c).length

// Fred in the middle with his owner beside him; every other circle is an island around them.
// An island's people sit on an oval, their names pointing outwards.
type Pos = { x: number; y: number; anchor: 'start' | 'end' | 'middle'; dx: number; dy: string }
const pos: Record<string, Pos> = {}
const CX = 468
const CY = 372
pos.fred = { x: CX, y: CY, anchor: 'middle', dx: 0, dy: '0.35em' }
pos.aleksei = { x: CX + 62, y: CY - 44, anchor: 'start', dx: 13, dy: '0.35em' }

// centre and size of each island's oval (the picture is 900 × 720); start = angle of its first name
const PLACES: Record<Exclude<GraphCluster, 'self'>, { x: number; y: number; rx: number; ry: number; start?: number }> = {
  friends: { x: 178, y: 372, rx: 74, ry: 212 },
  family: { x: 468, y: 112, rx: 92, ry: 36, start: Math.PI },
  aengix: { x: 742, y: 196, rx: 44, ry: 62 },
  ukraine: { x: 770, y: 382, rx: 30, ry: 30, start: -Math.PI / 2 },
  peers: { x: 736, y: 566, rx: 44, ry: 52 },
  salsa: { x: 468, y: 624, rx: 96, ry: 30, start: Math.PI },
}
const islands = (Object.keys(PLACES) as (keyof typeof PLACES)[]).map((id) => {
  const p = PLACES[id]
  const inside = nodes.filter((n) => n.cluster === id)
  inside.forEach((n, i) => {
    // wide, flat islands (family, salsa) spread their people along the upper and lower arc
    const a = (p.start ?? -Math.PI / 2) + (2 * Math.PI * (i + 0.5)) / inside.length
    const x = p.x + p.rx * Math.cos(a)
    const y = p.y + p.ry * Math.sin(a)
    const side = Math.cos(a)
    const flat = p.rx > p.ry * 1.6
    const anchor = flat || Math.abs(side) < 0.2 ? 'middle' : side > 0 ? 'start' : 'end'
    const dx = anchor === 'start' ? 11 : anchor === 'end' ? -11 : 0
    const dy = anchor === 'middle' ? (Math.sin(a) < 0 ? '-0.9em' : '1.65em') : '0.35em'
    pos[n.id] = { x: Math.round(x), y: Math.round(y), anchor, dx, dy }
  })
  const circle = circles.find((c) => c.id === id)!
  // the island reaches past the names
  const padX = p.rx > p.ry * 1.6 ? 58 : 92
  return { id, title: circle.title, x: p.x, y: p.y, rx: p.rx + padX, ry: p.ry + 40 }
})

const near = computed(() => {
  const s = new Set<string>()
  for (const e of edges) {
    if (e.from === active.value) s.add(e.to)
    if (e.to === active.value) s.add(e.from)
  }
  return s
})
const focused = computed(() => !!activeCircle.value || active.value !== 'fred')
const inCircle = (id: string) => byId[id]?.cluster === activeCircle.value
const touches = (e: GraphEdge) =>
  activeCircle.value ? inCircle(e.from) || inCircle(e.to) : e.from === active.value || e.to === active.value
const dimNode = (n: GraphNode) =>
  activeCircle.value
    ? n.cluster !== activeCircle.value && n.id !== 'fred'
    : active.value !== 'fred' && n.id !== active.value && !near.value.has(n.id)
const dimCircle = (id: GraphCluster) =>
  activeCircle.value ? activeCircle.value !== id : active.value !== 'fred' && current.value.cluster !== id && ![...near.value].some((x) => byId[x]?.cluster === id)
// lines from Fred carry the colour of the other end's circle; the rest is the web around him
function edgeTone(e: GraphEdge) {
  if (e.from !== 'fred' && e.to !== 'fred') return e.kind === 'peer' || e.kind === 'bia' ? 'peers' : 'around'
  return byId[e.from === 'fred' ? e.to : e.from]?.cluster ?? 'friends'
}
const radius = (n: GraphNode) => (n.id === 'fred' ? 27 : n.id === 'aleksei' ? 9 : n.kind === 'person' ? 6 : 8)
const kindLabel = (n: GraphNode) =>
  n.id === 'fred' ? t('graph:kind_self') : n.kind === 'bia' ? t('graph:kind_bia') : n.kind === 'agent' ? t('graph:kind_agent') : n.id === 'aleksei' ? t('graph:kind_owner') : (circles.find((c) => c.id === n.cluster)?.title ?? '')
const CHANNELS: Record<string, string> = {
  web: t('graph:ch_web'), telegram: 'Telegram', whatsapp: 'WhatsApp', instagram: 'Instagram', messenger: 'Messenger',
  max: 'MAX', linkedin: 'LinkedIn', calls: t('graph:ch_calls'), email: t('graph:ch_email'), peer: t('graph:ch_peer'),
}
const channelName = (c: string) => CHANNELS[c] ?? c
</script>

<style scoped>
.sg {
  margin: 0;
  display: grid;
  gap: 1.5rem;
  grid-template-columns: minmax(0, 1.9fr) minmax(15rem, 1fr);
  align-items: center;
  /* one colour per circle */
  --c-family: #c79a5c;
  --c-friends: #3f6f2a;
  --c-salsa: #c8553d;
  --c-aengix: #3d6ea8;
  --c-ukraine: #2a8c82;
  --c-peers: #7a4fb0;
}
.sg__svg {
  width: 100%;
  height: auto;
  overflow: visible;
  font-family: var(--font-body);
}
.sg__isle { cursor: pointer; transition: opacity 0.2s ease; }
.sg__isle ellipse { fill: var(--c); fill-opacity: 0.07; stroke: var(--c); stroke-opacity: 0.28; stroke-width: 1.2; stroke-dasharray: 4 5; transition: fill-opacity 0.2s ease; }
.sg__isle.is-on ellipse { fill-opacity: 0.14; stroke-opacity: 0.6; }
.sg__isle.is-dim { opacity: 0.45; }
.sg__isle-name { font-size: 12.5px; font-weight: 800; letter-spacing: 0.08em; text-transform: uppercase; fill: var(--c); }
.sg__isle--family, .sg__node--family, .sg__dot--family, .sg__edge--family { --c: var(--c-family); }
.sg__isle--friends, .sg__node--friends, .sg__dot--friends, .sg__edge--friends { --c: var(--c-friends); }
.sg__isle--salsa, .sg__node--salsa, .sg__dot--salsa, .sg__edge--salsa { --c: var(--c-salsa); }
.sg__isle--aengix, .sg__node--aengix, .sg__dot--aengix, .sg__edge--aengix { --c: var(--c-aengix); }
.sg__isle--ukraine, .sg__node--ukraine, .sg__dot--ukraine, .sg__edge--ukraine { --c: var(--c-ukraine); }
.sg__isle--peers, .sg__node--peers, .sg__dot--peers, .sg__edge--peers { --c: var(--c-peers); }
.sg__node--self, .sg__edge--self { --c: var(--c-family); }

.sg__edge { stroke: var(--c, rgba(29, 33, 38, 0.6)); stroke-width: 1.4; opacity: 0.5; transition: opacity 0.2s ease, stroke-width 0.2s ease; }
.sg__edge--around { stroke: rgba(29, 33, 38, 0.6); stroke-width: 1; opacity: 0.13; }
.sg__edge.is-on { stroke-width: 2.4; opacity: 1; }
.sg__edge--around.is-on { opacity: 0.55; stroke-width: 1.6; }
.sg__edge.is-dim { opacity: 0.07; }

.sg__node { cursor: pointer; outline: none; transition: opacity 0.2s ease; }
.sg__node circle { fill: color-mix(in srgb, var(--c) 16%, #fff); stroke: var(--c); stroke-width: 2; transition: r 0.15s ease; }
.sg__node text { font-size: 13px; fill: var(--ink-soft); font-weight: 600; paint-order: stroke; stroke: var(--paper, #fbf8f2); stroke-width: 3px; stroke-linejoin: round; }
.sg__node--bia:not(.sg__node--me) circle, .sg__node--agent circle { stroke-width: 3; }
.sg__node--me circle { fill: var(--accent); stroke: #fff; stroke-width: 4; filter: drop-shadow(0 6px 14px rgba(63, 111, 42, 0.35)); }
.sg__me { fill: #fff !important; stroke: none !important; font-family: var(--font-display); font-size: 16px !important; font-weight: 700 !important; }
.sg__node.is-on text, .sg__node:focus-visible text { fill: var(--ink); font-weight: 800; }
.sg__node.is-on:not(.sg__node--me) circle, .sg__node:focus-visible:not(.sg__node--me) circle { stroke-width: 3.5; }
.sg__node.is-dim { opacity: 0.3; }

.sg__info {
  background: var(--card);
  border: 1px solid var(--line);
  border-radius: var(--radius);
  box-shadow: var(--shadow);
  padding: 1.25rem 1.3rem;
  min-height: 17rem;
}
.sg__who { display: flex; align-items: baseline; gap: 0.6rem; flex-wrap: wrap; }
.sg__who strong { font-family: var(--font-display); font-size: 1.5rem; }
.sg__kind { color: var(--mute); font-size: 0.9rem; }
.sg__note { margin: 0.5rem 0 0.8rem; color: var(--ink-soft); min-height: 5.6em; }
.sg__chips { display: flex; flex-wrap: wrap; gap: 0.35rem; }
.sg__legend { list-style: none; margin: 1.1rem 0 0; padding: 0.9rem 0 0; border-top: 1px solid var(--line); display: flex; flex-wrap: wrap; gap: 0.3rem 0.5rem; }
.sg__legend button {
  display: inline-flex; align-items: center; gap: 0.4rem;
  appearance: none; background: none; border: 1px solid transparent; border-radius: 999px;
  padding: 0.2rem 0.55rem; font: inherit; font-size: 0.85rem; color: var(--mute); cursor: pointer;
}
.sg__legend button:hover, .sg__legend button.is-on { border-color: var(--line); color: var(--ink); background: var(--paper, #fbf8f2); }
.sg__dot { width: 0.7rem; height: 0.7rem; border-radius: 50%; border: 2px solid var(--c); background: color-mix(in srgb, var(--c) 16%, #fff); display: inline-block; }
.sg__small { margin: 0.8rem 0 0; font-size: 0.82rem; color: var(--mute); }

@media (max-width: 900px) {
  .sg { grid-template-columns: 1fr; }
  .sg__node text { font-size: 14px; }
  .sg__info { min-height: 0; }
}
@media (max-width: 640px) {
  /* thirty names do not fit a phone's width: the picture scrolls sideways */
  .sg__scroll { overflow-x: auto; -webkit-overflow-scrolling: touch; margin-inline: -1rem; padding-inline: 1rem; }
  .sg__svg { width: 640px; max-width: none; }
}
</style>
