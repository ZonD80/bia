import type { RouterConfig } from '@nuxt/schema'

// bia's app links carry data in the fragment (/open#ca=…&at=…&go=…). The default scroll
// behaviour turns any hash into a CSS selector; on that fragment the selector is invalid and
// the first navigation fails, so the page never hydrates. Only scroll to real element ids.
export default <RouterConfig>{
  scrollBehavior(to, _from, saved) {
    if (saved) return saved
    const id = to.hash.slice(1)
    if (id && /^[A-Za-z][\w-]*$/.test(id)) return { el: `#${CSS.escape(id)}`, behavior: 'smooth' }
    return { top: 0 }
  },
}
