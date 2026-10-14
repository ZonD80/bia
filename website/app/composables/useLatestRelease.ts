// The bia writer downloads, looked up in the GitHub releases of zond80/bia.
//
// tools/release.sh fills a release package by package and uses unversioned asset names
// (bia-flash-macos-universal.dmg, …), so the newest release may not have every writer yet.
// Each host gets the asset from the newest published release that has it; until the
// check finishes (or if it fails) the buttons point at the releases page.

const REPO = 'zond80/bia'
const RELEASES_API = `https://api.github.com/repos/${REPO}/releases?per_page=20`
const FALLBACK_RELEASE_PAGE = `https://github.com/${REPO}/releases/latest`

export type FlashHost = 'macos' | 'windows' | 'linux'

export const FLASH_ASSETS: Record<FlashHost, string> = {
  macos: 'bia-flash-macos-universal.dmg',
  windows: 'bia-flash-windows-x86_64.zip',
  linux: 'bia-flash-linux-x86_64.tar.gz',
}

type GitHubReleaseAsset = {
  name: string
  browser_download_url: string
}

type GitHubRelease = {
  tag_name: string
  html_url: string
  draft: boolean
  prerelease: boolean
  published_at: string | null
  assets: GitHubReleaseAsset[]
}

export type FlashDownload = {
  /** the asset itself, or the releases page when no release has it */
  url: string
  /** version of the release the asset comes from */
  version: string | null
  available: boolean
}

function versionParts(version: string): number[] {
  return version.split('.').map((part) => Number.parseInt(part, 10) || 0)
}

function compareVersions(left: string, right: string): number {
  const a = versionParts(left)
  const b = versionParts(right)
  const n = Math.max(a.length, b.length)
  for (let i = 0; i < n; i++) {
    const av = a[i] ?? 0
    const bv = b[i] ?? 0
    if (av !== bv) return av - bv
  }
  return 0
}

const tagVersion = (tag: string) => tag.replace(/^v/, '')

function unchecked(): Record<FlashHost, FlashDownload> {
  const out = {} as Record<FlashHost, FlashDownload>
  for (const host of Object.keys(FLASH_ASSETS) as FlashHost[]) {
    out[host] = { url: FALLBACK_RELEASE_PAGE, version: null, available: false }
  }
  return out
}

export function useLatestRelease() {
  const downloads = useState<Record<FlashHost, FlashDownload>>('bia-flash-downloads', unchecked)
  // Newest release that has the OS manifest bia-flash writes the stick from.
  const version = useState<string | null>('bia-flash-version', () => null)
  const releaseUrl = useState<string>('bia-flash-release-url', () => FALLBACK_RELEASE_PAGE)
  const loading = useState('bia-flash-loading', () => false)
  const error = useState<string | null>('bia-flash-error', () => null)
  const fetched = useState('bia-flash-fetched', () => false)

  async function refresh() {
    loading.value = true
    error.value = null
    try {
      const all = await $fetch<GitHubRelease[]>(RELEASES_API, {
        headers: {
          Accept: 'application/vnd.github+json',
          'X-GitHub-Api-Version': '2022-11-28',
        },
      })
      const releases = all
        .filter((r) => !r.draft && !r.prerelease)
        .sort((a, b) => compareVersions(tagVersion(b.tag_name), tagVersion(a.tag_name)))

      const next = unchecked()
      for (const host of Object.keys(FLASH_ASSETS) as FlashHost[]) {
        for (const r of releases) {
          const asset = r.assets.find((a) => a.name === FLASH_ASSETS[host])
          if (asset) {
            next[host] = { url: asset.browser_download_url, version: tagVersion(r.tag_name), available: true }
            break
          }
        }
      }
      downloads.value = next

      const os = releases.find((r) => r.assets.some((a) => a.name === 'bia-os.json'))
      version.value = os ? tagVersion(os.tag_name) : null
      releaseUrl.value = os?.html_url || FALLBACK_RELEASE_PAGE
      if (!Object.values(next).some((d) => d.available)) error.value = 'No release has a writer download yet'
      else if (!os) error.value = 'No release has the bia system yet'
    } catch {
      downloads.value = unchecked()
      error.value = 'Could not load the latest release'
    } finally {
      fetched.value = true
      loading.value = false
    }
  }

  async function ensureLoaded() {
    if (fetched.value || loading.value) return
    await refresh()
  }

  if (import.meta.client) {
    onMounted(() => {
      void ensureLoaded()
    })
  }

  return {
    downloads,
    version,
    releaseUrl,
    loading,
    error,
    fetched,
    refresh,
    ensureLoaded,
    fallbackUrl: FALLBACK_RELEASE_PAGE,
  }
}
