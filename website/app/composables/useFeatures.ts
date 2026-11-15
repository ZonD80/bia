import { featuresFor, type T } from '~/data/features'
import { buildOwnUses, buildUseCaseGroups } from '~/data/usecases'

/** Features, their groups and a lookup by slug, in the page's language. */
export function useFeatures() {
  return featuresFor(useNuxtApp().$i18n)
}

/** Who bia is for (needs the "usecases" translation file: home and /use-cases). */
export function useUseCases() {
  const t = useT() as unknown as T
  return { useCaseGroups: buildUseCaseGroups(t), ownUses: buildOwnUses(t) }
}
