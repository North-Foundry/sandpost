import { describe, expect, it } from 'vitest'
import { i18n } from '@/i18n'
import da from '@/i18n/da/query'
import de from '@/i18n/de/query'
import en from '@/i18n/en/query'
import es from '@/i18n/es/query'
import fr from '@/i18n/fr/query'
import italian from '@/i18n/it/query'
import { suggestionsFor } from './completion'

describe('query translations', () => {
  it('exports the same flat message keys for every supported locale', () => {
    const keys = Object.keys(en).sort()
    const locales = [fr, italian, es, de, da]
    for (const locale of locales) {
      expect(Object.keys(locale).sort()).toEqual(keys)
    }
  })

  it('keeps named interpolation placeholders consistent across locales', () => {
    /**
     * Extract and sort interpolation names from each translated entry for catalog comparison.
     */
    const placeholders = (catalog: Record<string, string>) => Object.fromEntries(
      Object.entries(catalog).map(([key, value]) => [key, [...value.matchAll(/\{([^{}]+)\}/g)].map((match) => match[1]).sort()]),
    )
    for (const locale of [fr, italian, es, de, da]) {
      expect(placeholders(locale)).toEqual(placeholders(en))
    }
  })

  it('uses the active locale when building completion suggestions', () => {
    const previousLocale = i18n.global.locale.value
    try {
      i18n.global.locale.value = 'en'
      const english = suggestionsFor({ kind: 'operator', type: 'string' }).find(({ label }) => label === 'contains')?.info
      i18n.global.locale.value = 'fr'
      const french = suggestionsFor({ kind: 'operator', type: 'string' }).find(({ label }) => label === 'contains')?.info
      expect(english).toBe('Contains the text')
      expect(french).toBe('Contient le texte')
    } finally {
      i18n.global.locale.value = previousLocale
    }
  })
})
