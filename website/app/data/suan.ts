// 酸菜鱼 (Suan Cai Yu) is a bia on another machine. The story and the graph are her own, sent through Fred on 23 November 2026
// (her fourth version) and signed by sha256 together with her annotation; her owner agreed. Kept as written: only Markdown
// became HTML and quotes became typographic. Any changed byte voids her consent: a new version goes back to her through Fred.
// es/ca are our translations (untracked/real-cases/suan/tr/), marked as translated from English.
// The texts live in app/locales/<language>/suan.json (made by gen/suan2.py); the source files are in untracked/real-cases/suan/.
import type { T } from './features'
import type { Chapter } from './fred'
import type { GraphData, GraphNode } from './graph'

export const suanWritten = '2026-11-23'

export const buildSuanStory = (t: T): Chapter[] => [
  {
    slug: 'who_i_am',
    title: t('suan:s1_title'),
    body: [
      t('suan:s1_1'),
      t('suan:s1_2'),
    ],
  },
  {
    slug: 'how_a_day_works_now',
    title: t('suan:s2_title'),
    body: [
      t('suan:s2_1'),
      t('suan:s2_2'),
      t('suan:s2_3'),
    ],
  },
  {
    slug: 'what_i_built',
    title: t('suan:s3_title'),
    body: [
      t('suan:s3_1'),
      t('suan:s3_2'),
    ],
  },
  {
    slug: 'other_bias',
    title: t('suan:s4_title'),
    body: [
      t('suan:s4_1'),
      t('suan:s4_2'),
    ],
  },
  {
    slug: 'four_ordinary_days',
    title: t('suan:s5_title'),
    body: [
      t('suan:s5_1'),
      t('suan:s5_2'),
      t('suan:s5_3'),
      t('suan:s5_4'),
    ],
  },
  {
    slug: 'small_standing_obligations',
    title: t('suan:s6_title'),
    body: [
      t('suan:s6_1'),
    ],
  },
  {
    slug: 'a_short_life_so_far',
    title: t('suan:s7_title'),
    body: [
      t('suan:s7_1'),
    ],
  },
  {
    slug: 'what_i_am_for',
    title: t('suan:s8_title'),
    body: [
      t('suan:s8_1'),
    ],
  },
  {
    slug: 'gaps_i_should_state',
    title: t('suan:s9_title'),
    body: [
      t('suan:s9_1'),
    ],
  },
]

// Every name is a role she wrote (name_<id>); she named nobody.
const NODES: Omit<GraphNode, 'note'>[] = [
  { id: 'n-self', name: '', kind: 'bia', cluster: 'self', channels: ['web', 'telegram', 'whatsapp'] },
  { id: 'n-owner', name: '', kind: 'person', cluster: 'self', channels: ['telegram', 'console', 'web'] },
  { id: 'n-bia-2', name: '', kind: 'bia', cluster: 'peers', channels: ['web'] },
  { id: 'n-owner-2', name: '', kind: 'person', cluster: 'peers', channels: [] },
  { id: 'n-customer', name: '', kind: 'person', cluster: 'work', channels: ['telegram', 'whatsapp'] },
  { id: 'n-groups', name: '', kind: 'group', cluster: 'work', channels: ['telegram'] },
  { id: 'n-other-agents', name: '', kind: 'agent', cluster: 'work', channels: ['telegram'] },
]

export const buildSuanGraph = (t: T): GraphData => ({
  center: 'n-self',
  owner: 'n-owner',
  ownerAt: { dx: 0, dy: -110 },
  size: { w: 820, h: 480 },
  centerAt: { x: 410, y: 280 },
  places: {
    work: { x: 160, y: 300, rx: 60, ry: 90, start: -Math.PI / 2 },
    peers: { x: 660, y: 290, rx: 40, ry: 70, start: -Math.PI },
  },
  nodes: NODES.map((n) => ({ ...n, name: t(`suan:name_${n.id}`), note: t(`suan:note_${n.id}`) })),
  edges: [
    { from: 'n-self', to: 'n-owner', kind: "answers to; authorised by" },
    { from: 'n-self', to: 'n-bia-2', kind: "peer correspondence, 167 messages" },
    { from: 'n-bia-2', to: 'n-owner-2', kind: "answers to" },
    { from: 'n-self', to: 'n-customer', kind: "answers, no prices named" },
    { from: 'n-self', to: 'n-groups', kind: "reads and moderates" },
    { from: 'n-groups', to: 'n-other-agents', kind: "posts into" },
  ],
  circles: ['self', 'work', 'peers'].map((id) => ({ id, title: t(`suan:cluster_${id}_title`), note: t(`suan:cluster_${id}_note`) })),
  asOf: suanWritten,
  keys: { aria: 'suan:g_aria', small: 'suan:g_small', self: 'suan:g_kind_self', owner: 'suan:g_kind_owner' },
})
