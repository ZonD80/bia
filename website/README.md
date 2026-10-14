# bia website

Static Nuxt site for people who are not technical: what bia is, every feature on its own page,
use cases, how it works, FAQ and the stick writer download.

- Content lives in `app/data/features.ts` (one entry per feature → `/features/<slug>`) and
  `app/data/usecases.ts` (→ `/use-cases`). Add a feature there and it appears everywhere
  (home, features index, footer, sitemap, `llms.txt`).
- The potato (`app/components/PotatoMark.vue`) is hand-drawn SVG; `public/icon.svg` is the favicon copy.
- `./dev.sh` runs the dev server; `npx nuxt generate` builds `.output/public`; `./deploy.sh` publishes
  to Cloudflare Pages.
- The download page asks the GitHub API for the releases of `zond80/bia` in the browser
  (`app/composables/useLatestRelease.ts`): each writer button gets its asset from the newest
  release that has it (`tools/release.sh` fills releases package by package), and the version
  shown is the newest release with `bia-os.json`. Until then, or if GitHub fails, the buttons
  open the releases page.
- `NUXT_PUBLIC_SITE_URL` sets canonical URLs in the sitemap.
- Old addresses (`/difference`, `/included/*`) redirect to the new pages.
