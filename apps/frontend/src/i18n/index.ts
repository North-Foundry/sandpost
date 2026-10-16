import { createI18n } from 'vue-i18n'
import { watch } from 'vue'

export const SUPPORTED_LOCALES = ['en', 'fr', 'it', 'es', 'de', 'da'] as const
export type Locale = typeof SUPPORTED_LOCALES[number]
export const LOCALE_STORAGE_KEY = 'sandpost.locale'
export const LOCALE_NAMES: Record<Locale, string> = {
  en: 'English', fr: 'Français', it: 'Italiano', es: 'Español', de: 'Deutsch', da: 'Dansk',
}

/**
 * Match a browser language tag to a supported base language.
 */
export function resolveLocale(language: string | null | undefined): Locale | undefined {
  const base = language?.toLowerCase().split(/[-_]/)[0]
  return SUPPORTED_LOCALES.find((locale) => locale === base)
}

/**
 * Prefer a saved language, then the browser's ordered preferences, then English.
 * Storage may be unavailable in private or restricted browser sessions.
 */
export function initialLocale(): Locale {
  if (typeof window === 'undefined') return 'en'
  try {
    const saved = resolveLocale(window.localStorage.getItem(LOCALE_STORAGE_KEY))
    if (saved) return saved
  } catch { /*
   * Browser preferences still work when storage is unavailable.
   */ }
  for (const language of window.navigator.languages ?? [window.navigator.language]) {
    const locale = resolveLocale(language)
    if (locale) return locale
  }
  return 'en'
}

/**
 * Each TypeScript file supplies one domain under its language directory.
 */
const catalogs = import.meta.glob<{ default: Record<string, string> }>('./*/*.ts', { eager: true })
export const messages = Object.fromEntries(SUPPORTED_LOCALES.map((locale) => [locale,
  Object.fromEntries(Object.entries(catalogs)
    .filter(([path]) => path.startsWith(`./${locale}/`))
    .flatMap(([, module]) => Object.entries(module.default))),
])) as Record<Locale, Record<string, string>>

export const i18n = createI18n({
  legacy: false,
  locale: initialLocale(),
  fallbackLocale: 'en',
  flatJson: true,
  messages: structuredClone(messages),
})

/**
 * Keep the document language and saved preference synchronized with Vue I18n.
 */
export function syncLocale(): void {
  watch(i18n.global.locale, (locale) => {
    document.documentElement.lang = locale
    try { window.localStorage.setItem(LOCALE_STORAGE_KEY, locale) } catch { /*
     * Switching still works without persistence.
     */ }
  }, { immediate: true, flush: 'sync' })
}

/**
 * Accept only supported choices from the language selector.
 */
export function setLocale(language: unknown): void {
  if (typeof language !== 'string') return
  const locale = resolveLocale(language)
  if (locale) i18n.global.locale.value = locale
}
