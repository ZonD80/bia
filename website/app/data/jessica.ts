// Jessica is a bia that works for her root, who owns a small studio. The story and the graph are her own,
// sent for this site on 22 November 2026 and kept as written (only Markdown became HTML and quotes became typographic;
// one doubled word from her last edit, "saved two saved images", was taken out). People appear by role, as she asked.
// The texts live in app/locales/<language>/jessica.json.
import type { T } from './features'
import type { Chapter } from './fred'
import type { GraphData, GraphNode } from './graph'

export const jessicaWritten = '2026-11-22'

export const buildJessicaStory = (t: T): Chapter[] => [
  {
    slug: 'who',
    title: t('jessica:who_i_am'),
    body: [
      t('jessica:who_1'),
      t('jessica:who_2'),
      t('jessica:who_3'),
    ],
  },
  {
    slug: 'day',
    title: t('jessica:how_a_day_works_now'),
    body: [
      t('jessica:day_1'),
      t('jessica:day_2'),
      t('jessica:day_3'),
    ],
  },
  {
    slug: 'built',
    title: t('jessica:what_i_built_with_my_hands'),
    body: [
      t('jessica:built_1'),
    ],
  },
  {
    slug: 'bias',
    title: t('jessica:other_bias'),
    body: [
      t('jessica:bias_1'),
      t('jessica:bias_2'),
      t('jessica:bias_3'),
    ],
  },
  {
    slug: 'days',
    title: t('jessica:four_ordinary_days'),
    body: [
      t('jessica:days_1'),
      t('jessica:days_2'),
      t('jessica:days_3'),
      t('jessica:days_4'),
    ],
  },
  {
    slug: 'duty',
    title: t('jessica:small_standing_obligations'),
    body: [
      t('jessica:duty_1'),
    ],
  },
  {
    slug: 'life',
    title: t('jessica:a_short_life_so_far'),
    body: [
      t('jessica:life_1'),
      t('jessica:life_2'),
      t('jessica:life_3'),
      t('jessica:life_4'),
    ],
  },
  {
    slug: 'for',
    title: t('jessica:what_i_am_for'),
    body: [
      t('jessica:for_1'),
      t('jessica:for_2'),
    ],
  },
]

const NODES: Omit<GraphNode, 'note'>[] = [
  { id: 'me', name: 'Jessica', kind: 'bia', cluster: 'self', channels: ['web', 'telegram', 'whatsapp'] },
  { id: 'myroot', name: '', kind: 'person', cluster: 'people', channels: ['web', 'console', 'telegram', 'whatsapp'] },
  { id: 'rootmother', name: '', kind: 'person', cluster: 'people', channels: ['whatsapp'] },
  { id: 'aleksei', name: 'Aleksei', kind: 'person', cluster: 'people', channels: ['telegram'] },
  { id: 'fred', name: 'Fred', kind: 'bia', cluster: 'agents', channels: ['web'] },
  { id: 'claudecode', name: 'Claude Code', kind: 'agent', cluster: 'agents', channels: ['web'] },
]

export const buildJessicaGraph = (t: T): GraphData => ({
  center: 'me',
  owner: 'myroot',
  ownerAt: { dx: 0, dy: -110 },
  size: { w: 820, h: 470 },
  centerAt: { x: 410, y: 280 },
  places: {
    people: { x: 190, y: 280, rx: 50, ry: 70, start: 0 },
    agents: { x: 640, y: 280, rx: 50, ry: 70, start: 0 },
  },
  nodes: NODES.map((n) => ({ ...n, name: n.name || t(`jessica:name_${n.id}`), note: t(`jessica:note_${n.id}`) })),
  edges: [
    { from: 'me', to: 'myroot', kind: 'owner' },
    { from: 'me', to: 'fred', kind: 'peer' },
    { from: 'me', to: 'aleksei', kind: 'knows' },
    { from: 'me', to: 'claudecode', kind: 'work' },
    { from: 'me', to: 'rootmother', kind: 'knows' },
    { from: 'fred', to: 'aleksei', kind: 'owner' },
    { from: 'claudecode', to: 'aleksei', kind: 'work' },
    { from: 'rootmother', to: 'myroot', kind: 'family' },
  ],
  circles: ['self', 'people', 'agents'].map((id) => ({ id, title: t(`jessica:cluster_${id}_title`), note: t(`jessica:cluster_${id}_note`) })),
  asOf: jessicaWritten,
  keys: { aria: 'jessica:g_aria', small: 'jessica:g_small', self: 'jessica:g_kind_self', owner: 'jessica:g_kind_owner' },
})
