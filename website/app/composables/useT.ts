import { useTranslation } from 'i18next-vue'

/** The translate function for scripts: t('pricing:title', { price }). Templates use $t. */
export function useT() {
  return useTranslation().t
}

/** A whole list or object of texts from a translation file: tm<Item[]>('faq:items'). */
export function useTm() {
  const { t } = useTranslation()
  return <T>(key: string, vars?: Record<string, unknown>) => t(key, { returnObjects: true, ...vars }) as unknown as T
}

/** Site paths in the current language: lp('/pricing') → /es/pricing. Templates use $lp. */
export function useLocalePath() {
  return useNuxtApp().$lp
}

export function useLocale() {
  return useNuxtApp().$locale
}
