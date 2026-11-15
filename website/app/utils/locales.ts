// The site's languages. English lives at the root (/pricing); the others under a prefix (/es/pricing).
// Texts are i18next JSON files in app/locales/<language>/<namespace>.json (see app/plugins/i18n.ts).

export const LOCALES = ['en', 'es', 'ca'] as const
export type Locale = (typeof LOCALES)[number]
export const DEFAULT_LOCALE: Locale = 'en'
export const LOCALE_NAMES: Record<Locale, string> = { en: 'English', es: 'Español', ca: 'Català' }
export const OG_LOCALES: Record<Locale, string> = { en: 'en_US', es: 'es_ES', ca: 'ca_ES' }
// where a person's own choice of language is kept (localStorage)
export const LOCALE_KEY = 'lang'

const PREFIX = new RegExp(`^/(${LOCALES.filter((l) => l !== DEFAULT_LOCALE).join('|')})(?=/|$)`)

/** The language a path is written in: /es/pricing → es, /pricing → en. */
export function localeOf(path: string): Locale {
  return (PREFIX.exec(path)?.[1] as Locale) || DEFAULT_LOCALE
}

/** The path without its language: /es/pricing → /pricing, /es → /. */
export function stripLocale(path: string): string {
  return path.replace(PREFIX, '').replace(/^(?=$|[?#])/, '/')
}

/** A site path in a language: ('/pricing#byok', 'es') → /es/pricing#byok. Other addresses pass through. */
export function localePath(path: string, locale: Locale): string {
  if (!path.startsWith('/') || path.startsWith('//')) return path
  const bare = stripLocale(path)
  return locale === DEFAULT_LOCALE ? bare : `/${locale}${bare}`
}

// Runs in the <head> before anything is drawn: a first visit to an English address moves to the
// person's language, when the site has it. Their own choice (the language menu) wins over the
// browser's languages; an address that already names a language is left alone.
export const DETECT_SCRIPT = `(function(){try{var L=${JSON.stringify(LOCALES.filter((l) => l !== DEFAULT_LOCALE))},p=location.pathname;if(/\\.[a-z0-9]+$/i.test(p)||new RegExp('^/('+L.join('|')+')(/|$)').test(p))return;var s=localStorage.getItem('${LOCALE_KEY}'),c=s?[s]:(navigator.languages||[navigator.language||'']);for(var i=0;i<c.length;i++){var l=String(c[i]).toLowerCase().split('-')[0];if(l==='${DEFAULT_LOCALE}')return;if(L.indexOf(l)>=0){location.replace('/'+l+p+location.search+location.hash);return}}}catch(e){}})()`

/** A date as the language writes it: ('2026-10-28', 'es') → 28 de octubre de 2026. */
export function longDate(iso: string, locale: Locale): string {
  return new Intl.DateTimeFormat(locale === 'en' ? 'en-GB' : locale, { day: 'numeric', month: 'long', year: 'numeric', timeZone: 'UTC' }).format(new Date(iso))
}
