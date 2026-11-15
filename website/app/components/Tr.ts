// A translated sentence with markup inside: links, <strong>, <br>…
// The text marks them by number — "Read the <0>terms</0> and <1>refunds</1>" — and the
// elements are given as children, in the same order:
//   <Tr k="home:read" :v="{ days }"><NuxtLink :to="$lp('/terms')" /><NuxtLink :to="$lp('/refunds')" /></Tr>
import { defineComponent, h, type VNode } from 'vue'
import { useTranslation } from 'i18next-vue'

type Part = string | VNode

export default defineComponent({
  name: 'Tr',
  props: {
    k: { type: String, required: true },
    v: { type: Object, default: undefined },
  },
  setup(props, { slots }) {
    const { t } = useTranslation()
    return () => {
      const text = String(t(props.k, { ...(props.v || {}) }))
      const marks = (slots.default?.() ?? []).filter((n) => typeof n.type !== 'symbol')
      const tokens = text.split(/(<\d+\/>|<\/?\d+>)/)
      let i = 0
      const read = (until?: string): Part[] => {
        const out: Part[] = []
        while (i < tokens.length) {
          const tok = tokens[i++]!
          const m = /^<(\/?)(\d+)(\/?)>$/.exec(tok)
          if (!m) {
            if (tok) out.push(tok)
          } else if (m[1]) {
            if (m[2] === until) return out
          } else {
            const mark = marks[Number(m[2])]
            const kids = m[3] ? [] : read(m[2])
            if (!mark) out.push(...kids)
            else if (m[3]) out.push(mark)
            else {
              const node = h(mark.type as any, mark.props, typeof mark.type === 'string' ? kids : { default: () => kids })
              // keep the parent's scoped styles on the element
              node.scopeId = mark.scopeId
              ;(node as any).slotScopeIds = (mark as any).slotScopeIds
              out.push(node)
            }
          }
        }
        return out
      }
      return read()
    }
  },
})
