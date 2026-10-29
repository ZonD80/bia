// The bia deploy downloads, looked up in the GitHub releases of zond80/bia.
//
// tools/release.sh fills a release package by package and uses unversioned asset names
// (bia-deploy-macos-universal.dmg, …), so the newest release may not have every download yet.
// Each host gets the asset from the newest published release that has it; until the
// check finishes (or if it fails) the buttons point at the releases page.

const REPO = 'zond80/bia'
/** Source code, issues and releases. */
export const REPO_URL = `https://github.com/${REPO}`
const RELEASES_API = `https://api.github.com/repos/${REPO}/releases?per_page=20`
const FALLBACK_RELEASE_PAGE = `https://github.com/${REPO}/releases/latest`

export type FlashHost = 'macos' | 'windows' | 'linux'

export const FLASH_ASSETS: Record<FlashHost, string> = {
  macos: 'bia-deploy-macos-universal.dmg',
  windows: 'bia-deploy-windows-x86_64.zip',
  linux: 'bia-deploy-linux-x86_64.tar.gz',
}

export type LandArch = 'x86_64' | 'arm64'

/** The land installer ISOs (xz-compressed: the raw ISO is over GitHub's 2 GiB file limit). */
export const LAND_ASSETS: Record<LandArch, string> = {
  x86_64: 'bia-land-x86_64.iso.xz',
  arm64: 'bia-land-arm64.iso.xz',
}

export type AppPlatform = 'android'

/** The phone app outside the stores (the APK is signed with the same key as the Play one). */
export const APP_ASSETS: Record<AppPlatform, string> = {
  android: 'bia-deploy-android.apk',
}

type GitHubReleaseAsset = {
  name: string
  browser_download_url: string
  size: number
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
  /** download size in bytes, when known */
  size?: number
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

function unchecked<K extends string>(assets: Record<K, string>): Record<K, FlashDownload> {
  const out = {} as Record<K, FlashDownload>
  for (const key of Object.keys(assets) as K[]) {
    out[key] = { url: FALLBACK_RELEASE_PAGE, version: null, available: false }
  }
  return out
}

/** Each asset from the newest release that has it. */
function pick<K extends string>(assets: Record<K, string>, releases: GitHubRelease[]): Record<K, FlashDownload> {
  const out = unchecked(assets)
  for (const key of Object.keys(assets) as K[]) {
    for (const r of releases) {
      const asset = r.assets.find((a) => a.name === assets[key])
      if (asset) {
        out[key] = {
          url: asset.browser_download_url,
          version: tagVersion(r.tag_name),
          available: true,
          size: asset.size,
        }
        break
      }
    }
  }
  return out
}

export function useLatestRelease() {
  const downloads = useState<Record<FlashHost, FlashDownload>>('bia-deploy-downloads', () => unchecked(FLASH_ASSETS))
  const landDownloads = useState<Record<LandArch, FlashDownload>>('bia-land-downloads', () => unchecked(LAND_ASSETS))
  const appDownloads = useState<Record<AppPlatform, FlashDownload>>('bia-app-downloads', () => unchecked(APP_ASSETS))
  // Newest release that has the OS manifest bia-deploy writes the stick from.
  const version = useState<string | null>('bia-deploy-version', () => null)
  const releaseUrl = useState<string>('bia-deploy-release-url', () => FALLBACK_RELEASE_PAGE)
  const loading = useState('bia-deploy-loading', () => false)
  const error = useState<string | null>('bia-deploy-error', () => null)
  const fetched = useState('bia-deploy-fetched', () => false)

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

      const next = pick(FLASH_ASSETS, releases)
      downloads.value = next
      landDownloads.value = pick(LAND_ASSETS, releases)
      appDownloads.value = pick(APP_ASSETS, releases)

      const os = releases.find((r) => r.assets.some((a) => a.name === 'bia-os.json'))
      version.value = os ? tagVersion(os.tag_name) : null
      releaseUrl.value = os?.html_url || FALLBACK_RELEASE_PAGE
      if (!Object.values(next).some((d) => d.available)) error.value = 'No release has a bia deploy download yet'
      else if (!os) error.value = 'No release has the bia system yet'
    } catch {
      downloads.value = unchecked(FLASH_ASSETS)
      landDownloads.value = unchecked(LAND_ASSETS)
      appDownloads.value = unchecked(APP_ASSETS)
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
    landDownloads,
    appDownloads,
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
