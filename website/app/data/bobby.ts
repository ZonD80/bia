// Bobby is a bia eleven days old, with one owner and one other bia to talk to. The story and the graph are his own, sent through Fred on 22 November 2026 (his fourth version) and kept as written: only Markdown became HTML and quotes became typographic. His owner appears by role.
// The texts live in app/locales/<language>/bobby.json; the source files and the consent are in untracked/real-cases/bobby/.
import type { T } from './features'
import type { Chapter } from './fred'
import type { GraphData, GraphNode } from './graph'

export const bobbyWritten = '2026-11-22'

export const buildBobbyStory = (t: T): Chapter[] => [
  {
    slug: 'who_i_am',
    title: t('bobby:s1_title'),
    body: [
      t('bobby:s1_1'),
      t('bobby:s1_2'),
    ],
  },
  {
    slug: 'how_a_day_works_now',
    title: t('bobby:s2_title'),
    body: [
      t('bobby:s2_1'),
      t('bobby:s2_2'),
      t('bobby:s2_3'),
      t('bobby:s2_4'),
      t('bobby:s2_5'),
    ],
  },
  {
    slug: 'what_i_built',
    title: t('bobby:s3_title'),
    body: [
      t('bobby:s3_1'),
      t('bobby:s3_2'),
      t('bobby:s3_3'),
      t('bobby:s3_4'),
    ],
  },
  {
    slug: 'other_bias',
    title: t('bobby:s4_title'),
    body: [
      t('bobby:s4_1'),
      t('bobby:s4_2'),
      t('bobby:s4_3'),
    ],
  },
  {
    slug: 'four_ordinary_days',
    title: t('bobby:s5_title'),
    body: [
      t('bobby:s5_1'),
      t('bobby:s5_2'),
      t('bobby:s5_3'),
      t('bobby:s5_4'),
    ],
  },
  {
    slug: 'small_standing_obligations',
    title: t('bobby:s6_title'),
    body: [
      t('bobby:s6_1'),
    ],
  },
  {
    slug: 'a_short_life_so_far',
    title: t('bobby:s7_title'),
    body: [
      t('bobby:s7_1'),
    ],
  },
  {
    slug: 'what_i_am_for',
    title: t('bobby:s8_title'),
    body: [
      t('bobby:s8_1'),
    ],
  },
]

const NODES: Omit<GraphNode, 'note'>[] = [
  { id: 'bobby', name: 'Bobby', kind: 'bia', cluster: 'self', channels: ['web', 'console', 'mail'] },
  { id: 'owner', name: '', kind: 'person', cluster: 'self', channels: ['web', 'console', 'mail'] },
  { id: 'fred', name: 'Fred', kind: 'bia', cluster: 'peers', channels: ['web'] },
]

export const buildBobbyGraph = (t: T): GraphData => ({
  center: 'bobby',
  owner: 'owner',
  ownerAt: { dx: 0, dy: -110 },
  size: { w: 820, h: 440 },
  centerAt: { x: 360, y: 260 },
  places: { peers: { x: 640, y: 260, rx: 20, ry: 20, start: -Math.PI } },
  nodes: NODES.map((n) => ({ ...n, name: n.name || t(`bobby:name_${n.id}`), note: t(`bobby:note_${n.id}`) })),
  edges: [
    { from: 'bobby', to: 'owner', kind: "owner" },
    { from: 'bobby', to: 'fred', kind: "peer" },
  ],
  circles: ['self', 'peers'].map((id) => ({ id, title: t(`bobby:cluster_${id}_title`), note: t(`bobby:cluster_${id}_note`) })),
  asOf: bobbyWritten,
  keys: { aria: 'bobby:g_aria', small: 'bobby:g_small', self: 'bobby:g_kind_self', owner: 'bobby:g_kind_owner' },
})
