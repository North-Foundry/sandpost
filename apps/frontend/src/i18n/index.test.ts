import { afterEach, describe, expect, it, vi } from 'vitest'
import { computed, effectScope } from 'vue'
import { createI18n } from 'vue-i18n'
import { formatBytes, formatFullDate } from '@/utils/format'
import { i18n, initialLocale, LOCALE_STORAGE_KEY, messages, resolveLocale, setLocale, SUPPORTED_LOCALES, syncLocale } from './index'

afterEach(() => {
  i18n.global.locale.value = 'en'
  vi.unstubAllGlobals()
  vi.restoreAllMocks()
})

describe('frontend languages', () => {
  it('resolves regional browser tags and rejects unsupported languages', () => {
    expect(resolveLocale('fr-CA')).toBe('fr')
    expect(resolveLocale('DE_de')).toBe('de')
    expect(resolveLocale('sv-SE')).toBeUndefined()
    expect(resolveLocale('constructor')).toBeUndefined()
    expect(resolveLocale(null)).toBeUndefined()
  })

  it('prefers saved choices, then supported browser languages, then English', () => {
    const getItem = vi.fn(() => 'it')
    vi.stubGlobal('window', { localStorage: { getItem }, navigator: { languages: ['sv', 'da-DK', 'de-DE'] } })
    expect(initialLocale()).toBe('it')
    getItem.mockReturnValue('unknown')
    expect(initialLocale()).toBe('da')
    getItem.mockImplementation(() => { throw new Error('Storage unavailable') })
    expect(initialLocale()).toBe('da')
    vi.stubGlobal('window', { localStorage: { getItem }, navigator: { languages: ['sv'] } })
    expect(initialLocale()).toBe('en')
  })

  it('persists switching, updates the document language and tolerates unavailable storage', () => {
    const setItem = vi.fn()
    vi.stubGlobal('window', { localStorage: { setItem } })
    vi.stubGlobal('document', { documentElement: { lang: '' } })
    const scope = effectScope()
    scope.run(syncLocale)
    try {
      setLocale('es')
      expect(i18n.global.locale.value).toBe('es')
      expect(document.documentElement.lang).toBe('es')
      expect(setItem).toHaveBeenLastCalledWith(LOCALE_STORAGE_KEY, 'es')
      setItem.mockImplementation(() => { throw new Error('Storage unavailable') })
      expect(() => setLocale('fr')).not.toThrow()
      expect(document.documentElement.lang).toBe('fr')
      setLocale('unsupported')
      setLocale(null)
      expect(i18n.global.locale.value).toBe('fr')
    } finally { scope.stop() }
  })

  it('reactively changes labels, dates and decimal separators', () => {
    const label = computed(() => i18n.global.t('common.inbox'))
    const date = new Date(2026, 9, 14, 13, 30).getTime()
    const english = formatFullDate(date)
    expect(label.value).toBe('Inbox')
    setLocale('it')
    expect(label.value).toBe('Posta in arrivo')
    expect(formatFullDate(date)).not.toBe(english)
    expect(formatBytes(1.5 * 1024 * 1024)).toBe('1,5 MB')
  })

  it('has complete matching catalogs and valid message syntax for every language', () => {
    const warnings = vi.spyOn(console, 'warn').mockImplementation(() => { })
    const errors = vi.spyOn(console, 'error').mockImplementation(() => { })
    const english = messages.en!
    for (const locale of SUPPORTED_LOCALES) {
      const catalog = messages[locale]!
      expect(Object.keys(catalog).sort(), locale).toEqual(Object.keys(english).sort())
      const translator = createI18n({ legacy: false, locale, fallbackLocale: false, flatJson: true, messages: { [locale]: { ...catalog } } }).global
      for (const [key, source] of Object.entries(english)) {
        const message = catalog[key]!
        expect(message.trim().length, `${locale}/${key}`).toBeGreaterThan(0)
        const parameters = Object.fromEntries([...source.matchAll(/\{(\w+)\}/g)].map((match) => [match[1]!, '2']))
        expect([...message.matchAll(/\{(\w+)\}/g)].map((match) => match[1]).sort(), `${locale}/${key}`)
          .toEqual([...source.matchAll(/\{(\w+)\}/g)].map((match) => match[1]).sort())
        const result = translator.t(key, parameters)
        expect(result, `${locale}/${key}`).not.toBe(key)
      }
    }
    expect(warnings.mock.calls).toEqual([])
    expect(errors.mock.calls).toEqual([])
  })
})
