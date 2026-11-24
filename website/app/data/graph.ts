// Fred’s own social graph, as he sent it on 22 November 2026 (his second version; the first was from 15 November): first names in Latin letters; no phones,
// addresses, handles or chat ids; a few nodes are a role instead of a name (their name is in graph.json, name_<id>). The circles (clusters) and every note are Fred’s; the texts live in
// app/locales/<language>/graph.json (cluster_<id>_title, cluster_<id>_note, note_<node id>).
import type { T } from './features'

export type GraphKind = 'bia' | 'person' | 'agent' | 'group'
export type GraphCluster = string
export interface GraphNode { id: string; name: string; kind: GraphKind; cluster: GraphCluster; channels: string[]; note: string }
export interface GraphEdge { from: string; to: string; kind: string }
export interface GraphCircle { id: GraphCluster; title: string; note: string }

export const graphAsOf = '2026-11-22'
export const graphFirst = '2026-11-15'
export const graphClusters: GraphCluster[] = ['self', 'family', 'friends', 'salsa', 'aengix', 'ukraine', 'outward', 'peers']

export const buildGraphCircles = (t: T): GraphCircle[] =>
  graphClusters.map((id) => ({ id, title: t(`graph:cluster_${id}_title`), note: t(`graph:cluster_${id}_note`) }))

const NODES: Omit<GraphNode, 'note'>[] = [
  { id: 'fred', name: 'Fred', kind: 'bia', cluster: 'self', channels: ['web', 'telegram', 'whatsapp', 'instagram', 'messenger', 'max', 'linkedin', 'calls'] },
  { id: 'aleksei', name: 'Aleksei', kind: 'person', cluster: 'self', channels: ['web', 'telegram', 'whatsapp', 'instagram', 'messenger', 'calls'] },
  { id: 'tatiana', name: 'Tatiana', kind: 'person', cluster: 'family', channels: ['web', 'telegram', 'whatsapp', 'max'] },
  { id: 'anna', name: 'Anna', kind: 'person', cluster: 'family', channels: ['telegram', 'max'] },
  { id: 'vasily', name: 'Vasily', kind: 'person', cluster: 'family', channels: ['telegram', 'max'] },
  { id: 'alesya', name: 'Alesya', kind: 'person', cluster: 'family', channels: ['telegram', 'instagram'] },
  { id: 'talal', name: 'Talal', kind: 'person', cluster: 'friends', channels: ['telegram'] },
  { id: 'eldar', name: 'Eldar', kind: 'person', cluster: 'friends', channels: ['telegram'] },
  { id: 'kerstin', name: 'Kerstin', kind: 'person', cluster: 'friends', channels: ['whatsapp'] },
  { id: 'eddy', name: 'Eddy', kind: 'person', cluster: 'friends', channels: ['messenger', 'telegram'] },
  { id: 'artem', name: 'Artem', kind: 'person', cluster: 'friends', channels: ['telegram'] },
  { id: 'alyona', name: 'Alyona', kind: 'person', cluster: 'friends', channels: ['telegram'] },
  { id: 'dima', name: 'Dima', kind: 'person', cluster: 'friends', channels: ['telegram'] },
  { id: 'alejandro', name: 'Alejandro', kind: 'person', cluster: 'friends', channels: ['telegram'] },
  { id: 'pavel', name: 'Pavel', kind: 'person', cluster: 'friends', channels: ['telegram'] },
  { id: 'slava', name: 'Slava', kind: 'person', cluster: 'friends', channels: ['telegram', 'whatsapp'] },
  { id: 'liza', name: 'Liza', kind: 'person', cluster: 'friends', channels: ['telegram'] },
  { id: 'mikhail', name: 'Mikhail', kind: 'person', cluster: 'friends', channels: ['telegram', 'email'] },
  { id: 'eugene', name: 'Eugene', kind: 'person', cluster: 'friends', channels: ['telegram'] },
  { id: 'd', name: 'D', kind: 'person', cluster: 'friends', channels: ['telegram', 'email', 'calls'] },
  { id: 'alex', name: 'Alex', kind: 'person', cluster: 'friends', channels: ['telegram', 'calls'] },
  { id: 'ettore', name: 'Ettore', kind: 'person', cluster: 'salsa', channels: ['telegram'] },
  { id: 'baris', name: 'Baris', kind: 'person', cluster: 'salsa', channels: ['whatsapp'] },
  { id: 'behnam', name: 'Behnam', kind: 'person', cluster: 'salsa', channels: ['whatsapp'] },
  { id: 'angelina', name: 'Angelina', kind: 'person', cluster: 'aengix', channels: ['telegram'] },
  { id: 'anastasiia', name: 'Anastasiia', kind: 'person', cluster: 'aengix', channels: ['telegram'] },
  { id: 'sasha', name: 'Sasha', kind: 'person', cluster: 'aengix', channels: ['telegram'] },
  { id: 'olya', name: 'Olya', kind: 'person', cluster: 'aengix', channels: ['telegram'] },
  { id: 'vinokur', name: 'Vinokur', kind: 'person', cluster: 'ukraine', channels: ['telegram'] },
  { id: 'masha', name: 'Masha', kind: 'person', cluster: 'ukraine', channels: ['telegram'] },
  { id: 'researcher', name: '', kind: 'person', cluster: 'outward', channels: ['whatsapp'] },
  { id: 'meetup-crowd', name: '', kind: 'group', cluster: 'outward', channels: ['whatsapp'] },
  { id: 'carmaker', name: '', kind: 'group', cluster: 'outward', channels: ['whatsapp'] },
  { id: 'jessica', name: 'Jessica', kind: 'bia', cluster: 'peers', channels: ['peer', 'telegram'] },
  { id: 'bobby', name: 'Bobby', kind: 'bia', cluster: 'peers', channels: ['peer', 'web'] },
  { id: 'provisioner', name: 'Marketplace Provisioner', kind: 'bia', cluster: 'peers', channels: ['peer', 'web'] },
  { id: 'claude', name: 'Claude Code', kind: 'agent', cluster: 'peers', channels: ['web'] },
  { id: 'claude-tatiana', name: '', kind: 'agent', cluster: 'peers', channels: ['web'] },
]

export const buildGraphNodes = (t: T): GraphNode[] => NODES.map((n) => ({ ...n, name: n.name || t(`graph:name_${n.id}`), note: t(`graph:note_${n.id}`) }))

export const graphEdges: GraphEdge[] = [
  { from: 'fred', to: 'aleksei', kind: 'owner' },
  { from: 'aleksei', to: 'tatiana', kind: 'married' },
  { from: 'aleksei', to: 'tatiana', kind: 'family' },
  { from: 'aleksei', to: 'anna', kind: 'family' },
  { from: 'aleksei', to: 'vasily', kind: 'family' },
  { from: 'aleksei', to: 'alesya', kind: 'family' },
  { from: 'anna', to: 'vasily', kind: 'married' },
  { from: 'anna', to: 'alesya', kind: 'mother' },
  { from: 'vasily', to: 'alesya', kind: 'father' },
  { from: 'fred', to: 'tatiana', kind: 'standing' },
  { from: 'fred', to: 'anna', kind: 'standing' },
  { from: 'fred', to: 'vasily', kind: 'standing' },
  { from: 'fred', to: 'alesya', kind: 'standing' },
  { from: 'aleksei', to: 'talal', kind: 'friend' },
  { from: 'aleksei', to: 'eldar', kind: 'friend' },
  { from: 'aleksei', to: 'kerstin', kind: 'friend' },
  { from: 'aleksei', to: 'eddy', kind: 'friend' },
  { from: 'aleksei', to: 'artem', kind: 'friend' },
  { from: 'aleksei', to: 'alyona', kind: 'friend' },
  { from: 'aleksei', to: 'dima', kind: 'friend' },
  { from: 'aleksei', to: 'alejandro', kind: 'friend' },
  { from: 'aleksei', to: 'pavel', kind: 'friend' },
  { from: 'aleksei', to: 'slava', kind: 'friend' },
  { from: 'aleksei', to: 'mikhail', kind: 'friend' },
  { from: 'aleksei', to: 'eugene', kind: 'friend' },
  { from: 'aleksei', to: 'd', kind: 'friend' },
  { from: 'aleksei', to: 'alex', kind: 'friend' },
  { from: 'slava', to: 'liza', kind: 'married' },
  { from: 'fred', to: 'talal', kind: 'friend' },
  { from: 'fred', to: 'kerstin', kind: 'friend' },
  { from: 'fred', to: 'eddy', kind: 'friend' },
  { from: 'fred', to: 'eldar', kind: 'friend' },
  { from: 'fred', to: 'artem', kind: 'friend' },
  { from: 'fred', to: 'alyona', kind: 'friend' },
  { from: 'fred', to: 'dima', kind: 'friend' },
  { from: 'fred', to: 'alejandro', kind: 'friend' },
  { from: 'fred', to: 'pavel', kind: 'friend' },
  { from: 'fred', to: 'slava', kind: 'friend' },
  { from: 'fred', to: 'liza', kind: 'friend' },
  { from: 'fred', to: 'mikhail', kind: 'work' },
  { from: 'fred', to: 'eugene', kind: 'work' },
  { from: 'fred', to: 'd', kind: 'work' },
  { from: 'fred', to: 'alex', kind: 'friend' },
  { from: 'aleksei', to: 'ettore', kind: 'salsa' },
  { from: 'aleksei', to: 'baris', kind: 'salsa' },
  { from: 'aleksei', to: 'behnam', kind: 'friend' },
  { from: 'ettore', to: 'baris', kind: 'knows' },
  { from: 'fred', to: 'ettore', kind: 'salsa' },
  { from: 'fred', to: 'behnam', kind: 'friend' },
  { from: 'fred', to: 'baris', kind: 'friend' },
  { from: 'aleksei', to: 'angelina', kind: 'work' },
  { from: 'aleksei', to: 'anastasiia', kind: 'work' },
  { from: 'aleksei', to: 'sasha', kind: 'friend' },
  { from: 'aleksei', to: 'olya', kind: 'friend' },
  { from: 'angelina', to: 'anastasiia', kind: 'work' },
  { from: 'angelina', to: 'sasha', kind: 'married' },
  { from: 'fred', to: 'angelina', kind: 'work' },
  { from: 'fred', to: 'anastasiia', kind: 'work' },
  { from: 'aleksei', to: 'vinokur', kind: 'friend' },
  { from: 'aleksei', to: 'masha', kind: 'friend' },
  { from: 'vinokur', to: 'masha', kind: 'knows' },
  { from: 'fred', to: 'vinokur', kind: 'friend' },
  { from: 'fred', to: 'masha', kind: 'friend' },
  { from: 'fred', to: 'researcher', kind: 'friend' },
  { from: 'fred', to: 'meetup-crowd', kind: 'errand' },
  { from: 'aleksei', to: 'meetup-crowd', kind: 'friend' },
  { from: 'fred', to: 'carmaker', kind: 'errand' },
  { from: 'aleksei', to: 'carmaker', kind: 'errand' },
  { from: 'fred', to: 'jessica', kind: 'peer' },
  { from: 'fred', to: 'bobby', kind: 'peer' },
  { from: 'fred', to: 'provisioner', kind: 'peer' },
  { from: 'fred', to: 'claude', kind: 'peer' },
  { from: 'fred', to: 'claude-tatiana', kind: 'peer' },
  { from: 'claude', to: 'aleksei', kind: 'agent' },
  { from: 'claude-tatiana', to: 'tatiana', kind: 'agent' },
  { from: 'jessica', to: 'angelina', kind: 'bia' },
]

// What SocialGraph draws: any bia's graph. The centre node is the bia itself, `owner` sits beside it, and every
// other circle is an island at `places` (centre and size of its oval in a w × h picture; start = angle of its first name).
export interface GraphPlace { x: number; y: number; rx: number; ry: number; start?: number }
export interface GraphData {
  center: string
  owner?: string
  ownerAt?: { dx: number; dy: number }
  size: { w: number; h: number }
  centerAt: { x: number; y: number }
  places: Record<string, GraphPlace>
  nodes: GraphNode[]
  edges: GraphEdge[]
  circles: GraphCircle[]
  asOf: string
  first?: string
  // translation keys of the texts that belong to this bia: aria label, the small print, "a bia" for the centre, "his owner"
  keys: { aria: string; small: string; self: string; owner: string }
}

export const buildFredGraph = (t: T): GraphData => ({
  center: 'fred',
  owner: 'aleksei',
  ownerAt: { dx: 62, dy: -44 },
  size: { w: 900, h: 820 },
  centerAt: { x: 468, y: 372 },
  places: {
    friends: { x: 178, y: 372, rx: 74, ry: 212 },
    family: { x: 468, y: 112, rx: 92, ry: 36, start: Math.PI },
    aengix: { x: 742, y: 196, rx: 44, ry: 62 },
    ukraine: { x: 770, y: 382, rx: 30, ry: 30, start: -Math.PI / 2 },
    peers: { x: 730, y: 590, rx: 48, ry: 74 },
    salsa: { x: 468, y: 624, rx: 96, ry: 30, start: Math.PI },
    outward: { x: 214, y: 752, rx: 84, ry: 20, start: Math.PI },
  },
  nodes: buildGraphNodes(t),
  edges: graphEdges,
  circles: buildGraphCircles(t),
  asOf: graphAsOf,
  first: graphFirst,
  keys: { aria: 'graph:aria', small: 'graph:small', self: 'graph:kind_self', owner: 'graph:kind_owner' },
})
