// Translations with i18next. One instance per app (so per request while the static pages are built);
// the language comes from the address, and each page loads only its own translation files.
import i18next from 'i18next'
import I18NextVue from 'i18next-vue'
import resourcesToBackend from 'i18next-resources-to-backend'

export default defineNuxtPlugin({
  name: 'i18n',
  enforce: 'pre',
  async setup(nuxtApp) {
    const path = import.meta.server ? useRequestURL().pathname : window.location.pathname
    const first = localeOf(path)
    const i18n = i18next.createInstance()
    await i18n
      .use(resourcesToBackend((lng: string, ns: string) => import(`../locales/${lng}/${ns}.json`)))
      .init({
        lng: first,
        supportedLngs: [...LOCALES],
        // every language is complete (check-locales.mjs), so no second language is downloaded as a fallback
        fallbackLng: false,
        ns: namespacesFor(path),
        defaultNS: 'common',
        interpolation: { escapeValue: false },
        saveMissing: true,
        missingKeyHandler(_lngs, ns, key) {
          // a missing text must stop the build, not ship as a raw key
          if (import.meta.server) throw new Error(`missing translation ${ns}:${key} (${i18n.language})`)
          console.warn(`missing translation ${ns}:${key} (${i18n.language})`)
        },
      })
    nuxtApp.vueApp.use(I18NextVue, { i18next: i18n })

    const locale = useState<Locale>('locale', () => first)
    locale.value = first
    // a site path, or a route object with one, in the current language
    function lp(to: string): string
    function lp<T extends { path: string }>(to: T): T
    function lp(to: string | { path: string }) {
      return typeof to === 'string' ? localePath(to, locale.value) : { ...to, path: localePath(to.path, locale.value) }
    }

    addRouteMiddleware(
      'i18n',
      async (to) => {
        const next = localeOf(to.path)
        if (next !== i18n.language) await i18n.changeLanguage(next)
        await i18n.loadNamespaces(namespacesFor(to.path))
        locale.value = next
      },
      { global: true },
    )

    // <html lang>, the same page in the other languages, and translated defaults for link previews
    const route = useRoute()
    const { siteUrl } = useRuntimeConfig().public
    const address = (l: Locale) => `${siteUrl}${localePath(stripLocale(route.path), l).replace(/\/$/, '')}/`
    const t = i18n.t.bind(i18n)
    useHead({
      htmlAttrs: { lang: () => locale.value },
      link: () =>
        [
          ...LOCALES.map((l) => ({ rel: 'alternate', hreflang: l, href: address(l) })),
          { rel: 'alternate', hreflang: 'x-default', href: address(DEFAULT_LOCALE) },
        ] as any[],
      meta: () => [
        { property: 'og:locale', content: OG_LOCALES[locale.value] },
        ...LOCALES.filter((l) => l !== locale.value).map((l) => ({ property: 'og:locale:alternate', content: OG_LOCALES[l] })),
      ],
    })
    useSeoMeta({
      title: () => (locale.value, t('common:meta.title')),
      description: () => (locale.value, t('common:meta.description')),
      ogTitle: () => (locale.value, t('common:meta.og_title')),
      ogDescription: () => (locale.value, t('common:meta.og_description')),
      ogImageAlt: () => (locale.value, t('common:meta.image_alt')),
    })

    return { provide: { lp, i18n, locale } }
  },
})
