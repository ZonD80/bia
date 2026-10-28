import { features } from './app/data/features'
import { useCaseGroups } from './app/data/usecases'
import { FRED_FILM } from './app/data/fred'

const siteUrl = (
  process.env.NUXT_PUBLIC_SITE_URL ||
  process.env.SITE_URL ||
  'https://ownbia.com'
).replace(/\/$/, '')

const prerenderRoutes = [
  '/',
  '/features',
  ...features.map((f) => `/features/${f.slug}`),
  '/use-cases',
  '/how-it-works',
  '/why-bia',
  '/fred',
  '/faq',
  '/download',
  '/servers',
  '/privacy',
]

// Old addresses keep working.
const oldIncluded = ['discover', 'shell-jobs', 'talk-calendar-memory', 'cloud-or-local', 'telegram', 'whatsapp', 'carry', 'chrome', 'secrets', 'screen']
const oldTarget: Record<string, string> = {
  discover: 'skills', 'shell-jobs': 'computer', 'talk-calendar-memory': 'memory', 'cloud-or-local': 'brain',
  telegram: 'messengers', whatsapp: 'messengers', carry: 'moving', chrome: 'browser', secrets: 'vault', screen: 'screen',
}
const redirects: Record<string, { redirect: string }> = {
  '/difference': { redirect: '/why-bia' },
  ...Object.fromEntries(oldIncluded.map((s) => [`/included/${s}`, { redirect: `/features/${oldTarget[s]}` }])),
}

function sitemapXml(urls: string[]) {
  const body = urls
    .map((path) => `  <url>\n    <loc>${siteUrl}${path}</loc>\n    <changefreq>weekly</changefreq>\n  </url>`)
    .join('\n')
  return `<?xml version="1.0" encoding="UTF-8"?>\n<urlset xmlns="http://www.sitemaps.org/schemas/sitemap/0.9">\n${body}\n</urlset>\n`
}

function robotsTxt() {
  return `User-Agent: *\nAllow: /\n\nSitemap: ${siteUrl}/sitemap.xml\n`
}

function llmsTxt() {
  return `# bia

> Boot into agent. Making every potato smart — on a global scale.

bia is a free operating system that boots on any computer or server — even an old laptop — and turns it into a helpful AI personality.
It starts from a USB stick or its own disk, lives on WhatsApp, Telegram, Facebook Messenger and Instagram as a real account, answers voice calls in about
100 languages, remembers, keeps a calendar, uses its own computer, browser and password vault, and can move
to other computers and update itself without forgetting anything. Its computer can also be a server or VPS: one
command replaces a running Linux with bia. Its data stays there, not in a company's cloud.

## Pages

- [Home](${siteUrl}/): what bia is, a day with bia, all features
- [Features](${siteUrl}/features): every ability, one page each
${features.map((f) => `- [${f.name}](${siteUrl}/features/${f.slug}): ${f.tagline}`).join('\n')}
- [Use cases](${siteUrl}/use-cases): ${useCaseGroups.map((g) => g.title.toLowerCase()).join('; ')}
- [How it works](${siteUrl}/how-it-works): plain-language overview
- [Why bia](${siteUrl}/why-bia): compared with typical AI assistants
- [Fred's story](${siteUrl}/fred): a real bia's first three weeks in its own words, and the unedited promo film it made (${FRED_FILM.url})
- [FAQ](${siteUrl}/faq)
- [Get bia](${siteUrl}/download): the USB stick writer
- [Privacy](${siteUrl}/privacy): no data collected, by the website, the bia deploy apps or bia
- [Servers and virtual machines](${siteUrl}/servers): turn a running Linux server or VPS into bia with one command (takeover.sh, also in land mode), let a bia set up a server over ssh and fly there, or install from the land ISO

## For AI agents

A running bia exposes a web app (chats, an MCP bus like Telegram) with a documented API at /.well-known/bia.json and /openapi.yaml
(client-certificate authentication; keys are issued by the bia's owner). Two bias can trust each other in one
request: GET /v1/pki/agent gives a bia's agent CSR, and a certificate request may carry a grant (a certificate
for the other side); one approval gives access both ways.

Take over a Linux server: curl -fsSL https://github.com/zond80/bia/releases/latest/download/takeover.sh | sudo bash
(flags and JSON configs: bia-deploy takeover --help; --land --land-secret <secret> makes a land a bia can fly to).
`
}

// https://nuxt.com/docs/api/configuration/nuxt-config
export default defineNuxtConfig({
  compatibilityDate: '2025-07-15',
  // Static HTML for every route so people, search engines and agents get real content.
  ssr: true,
  devtools: { enabled: false },
  css: ['~/assets/css/main.css'],
  runtimeConfig: {
    public: {
      siteUrl,
    },
  },
  routeRules: redirects,
  nitro: {
    preset: 'static',
    prerender: {
      crawlLinks: true,
      routes: [...prerenderRoutes, ...Object.keys(redirects)],
      failOnError: true,
    },
  },
  hooks: {
    'nitro:build:public-assets'(nitro) {
      const { writeFileSync } = require('node:fs') as typeof import('node:fs')
      const { join } = require('node:path') as typeof import('node:path')
      const dir = nitro.options.output.publicDir
      writeFileSync(join(dir, 'sitemap.xml'), sitemapXml(prerenderRoutes))
      writeFileSync(join(dir, 'robots.txt'), robotsTxt())
      writeFileSync(join(dir, 'llms.txt'), llmsTxt())
      // Real 301s on Cloudflare Pages; the prerendered meta-refresh pages are the fallback elsewhere.
      writeFileSync(join(dir, '_redirects'), Object.entries(redirects).map(([from, r]) => `${from} ${r.redirect} 301\n${from}/ ${r.redirect} 301`).join('\n') + '\n')
    },
  },
  app: {
    head: {
      title: 'bia — turn anything into someone',
      htmlAttrs: { lang: 'en' },
      meta: [
        {
          name: 'description',
          content:
            'bia is an operating system that turns any computer or server into a helpful mind: it lives on WhatsApp, Telegram, Messenger and Instagram, answers calls in 100 languages, remembers, keeps time, uses the web, and moves between computers as the same personality.',
        },
        { name: 'theme-color', content: '#fbf8f2' },
        { name: 'robots', content: 'index, follow' },
        { property: 'og:type', content: 'website' },
        { property: 'og:site_name', content: 'bia' },
        { property: 'og:title', content: 'bia — turn anything into someone' },
        { property: 'og:description', content: 'A helpful AI personality with a computer of its own, in your messengers, in your language.' },
        // The preview card for shared links (tools/make-og.sh); without it Threads and others
        // pick the first image on the page, the YouTube poster of Fred's film.
        { property: 'og:image', content: `${siteUrl}/og.png` },
        { property: 'og:image:type', content: 'image/png' },
        { property: 'og:image:width', content: '1200' },
        { property: 'og:image:height', content: '630' },
        { property: 'og:image:alt', content: 'bia: a potato wired to a glowing optic. Turn anything into someone.' },
        { name: 'twitter:card', content: 'summary_large_image' },
        { name: 'twitter:image', content: `${siteUrl}/og.png` },
      ],
      link: [
        { rel: 'icon', type: 'image/svg+xml', href: '/icon.svg' },
        { rel: 'alternate icon', href: '/favicon.ico' },
        { rel: 'apple-touch-icon', href: '/apple-touch-icon.png' },
        { rel: 'preconnect', href: 'https://fonts.googleapis.com' },
        { rel: 'preconnect', href: 'https://fonts.gstatic.com', crossorigin: '' },
        {
          rel: 'stylesheet',
          href: 'https://fonts.googleapis.com/css2?family=Fraunces:ital,opsz,wght@0,9..144,500;0,9..144,600;0,9..144,700;1,9..144,600&family=Manrope:wght@400;500;600;700&display=swap',
        },
      ],
    },
  },
})
