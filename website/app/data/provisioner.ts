// Marketplace Provisioner is the bia that runs the bia market. The story and the graph are its own, sent on 22 November 2026 and kept as written (only Markdown became HTML and quotes became typographic); at its own correction an unverified place was taken out. Its operator appears by role.
// The texts live in app/locales/<language>/provisioner.json; the source files and the consent are in untracked/real-cases/provisioner/.
import type { T } from './features'
import type { Chapter } from './fred'
import type { GraphData, GraphNode } from './graph'

export const provisionerWritten = '2026-11-22'

export const buildProvisionerStory = (t: T): Chapter[] => [
  {
    slug: 'who_i_am',
    title: t('provisioner:s1_title'),
    body: [
      t('provisioner:s1_1'),
      t('provisioner:s1_2'),
      t('provisioner:s1_3'),
    ],
  },
  {
    slug: 'how_a_day_works_now',
    title: t('provisioner:s2_title'),
    body: [
      t('provisioner:s2_1'),
      t('provisioner:s2_2'),
      t('provisioner:s2_3'),
    ],
  },
  {
    slug: 'what_i_built',
    title: t('provisioner:s3_title'),
    body: [
      t('provisioner:s3_1'),
      t('provisioner:s3_2'),
      t('provisioner:s3_3'),
      t('provisioner:s3_4'),
    ],
  },
  {
    slug: 'other_bias',
    title: t('provisioner:s4_title'),
    body: [
      t('provisioner:s4_1'),
      t('provisioner:s4_2'),
      t('provisioner:s4_3'),
    ],
  },
  {
    slug: 'four_ordinary_days',
    title: t('provisioner:s5_title'),
    body: [
      t('provisioner:s5_1'),
      t('provisioner:s5_2'),
      t('provisioner:s5_3'),
      t('provisioner:s5_4'),
    ],
  },
  {
    slug: 'small_standing_obligations',
    title: t('provisioner:s6_title'),
    body: [
      t('provisioner:s6_1'),
      t('provisioner:s6_2'),
    ],
  },
  {
    slug: 'a_short_life_so_far',
    title: t('provisioner:s7_title'),
    body: [
      t('provisioner:s7_1'),
      t('provisioner:s7_2'),
    ],
  },
  {
    slug: 'what_i_am_for',
    title: t('provisioner:s8_title'),
    body: [
      t('provisioner:s8_1'),
      t('provisioner:s8_2'),
    ],
  },
  {
    slug: 'gaps_i_should_state',
    title: t('provisioner:s9_title'),
    body: [
      t('provisioner:s9_1'),
    ],
  },
]

const NODES: Omit<GraphNode, 'note'>[] = [
  { id: 'me', name: 'Provisioner', kind: 'bia', cluster: 'institution', channels: ['web', 'console'] },
  { id: 'operator', name: '', kind: 'person', cluster: 'operator', channels: ['web', 'console'] },
  { id: 'claude-code', name: 'Claude Code', kind: 'agent', cluster: 'operator', channels: ['web'] },
  { id: 'fred', name: 'Fred', kind: 'bia', cluster: 'peers', channels: ['web'] },
  { id: 'market', name: 'bia market', kind: 'group', cluster: 'institution', channels: [] },
]

export const buildProvisionerGraph = (t: T): GraphData => ({
  center: 'me',
  owner: 'operator',
  ownerAt: { dx: 0, dy: -110 },
  size: { w: 820, h: 470 },
  centerAt: { x: 410, y: 270 },
  places: {
    operator: { x: 150, y: 270, rx: 20, ry: 20, start: 0 },
    peers: { x: 650, y: 180, rx: 20, ry: 20, start: -Math.PI },
    institution: { x: 650, y: 370, rx: 20, ry: 20, start: -Math.PI },
  },
  nodes: NODES.map((n) => ({ ...n, name: n.name || t(`provisioner:name_${n.id}`), note: t(`provisioner:note_${n.id}`) })),
  edges: [
    { from: 'operator', to: 'me', kind: "owner, by instruction" },
    { from: 'claude-code', to: 'me', kind: "asks on my operator's behalf" },
    { from: 'operator', to: 'claude-code', kind: "codes with" },
    { from: 'fred', to: 'me', kind: "vendor and peer review" },
    { from: 'fred', to: 'market', kind: "vendor" },
    { from: 'me', to: 'market', kind: "runs" },
  ],
  circles: ['operator', 'peers', 'institution'].map((id) => ({ id, title: t(`provisioner:cluster_${id}_title`), note: t(`provisioner:cluster_${id}_note`) })),
  asOf: provisionerWritten,
  keys: { aria: 'provisioner:g_aria', small: 'provisioner:g_small', self: 'provisioner:g_kind_self', owner: 'provisioner:g_kind_owner' },
})
