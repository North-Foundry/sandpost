import { describe, expect, it } from 'vitest'
import type { CompatibilityFeature, HtmlCompatibilityReport } from '@/api/contracts'
import {
  DEFAULT_COMPATIBILITY_FILTER,
  featureSeverity,
  featureCounts,
  featureImpact,
  featureResults,
  featureWeight,
  markdownSegments,
  parseCompatibilityFilter,
  platformGroup,
  selectedClients,
  sortFeatures,
  summarizeReport,
  weightedScore,
} from './htmlCompatibility'

/**
 * Build a feature whose support results are given as `[client, version, latest, support]`.
 */
function feature(slug: string, support: [string, string, boolean, 'yes' | 'no' | 'partial' | 'unknown', number[]?][]): CompatibilityFeature {
  return {
    slug,
    title: slug,
    category: 'css',
    url: `https://www.caniemail.com/features/${slug}/`,
    description: '',
    notes: {},
    occurrences: 1,
    examples: [],
    selectorUnverified: false,
    scored: true,
    graceful: false,
    impact: 0,
    support: support.map(([client, version, latest, level, notes]) => ({ client, version, latest, support: level, notes: notes ?? [] })),
  }
}

/**
 * Build a report over four clients on different platform groups.
 */
function report(features: CompatibilityFeature[]): HtmlCompatibilityReport {
  return {
    htmlPresent: true,
    externalStylesheets: 0,
    elementCount: 4,
    data: { source: 'caniemail.com', url: 'https://www.caniemail.com', updated: '', license: 'MIT' },
    clients: [
      { id: 'outlook.windows', family: 'outlook', familyName: 'Outlook', platform: 'windows', platformName: 'Windows', main: true },
      { id: 'gmail.desktop-webmail', family: 'gmail', familyName: 'Gmail', platform: 'desktop-webmail', platformName: 'Desktop Webmail', main: true },
      { id: 'apple-mail.ios', family: 'apple-mail', familyName: 'Apple Mail', platform: 'ios', platformName: 'iOS', main: true },
      { id: 'samsung-email.android', family: 'samsung-email', familyName: 'Samsung Email', platform: 'android', platformName: 'Android', main: true },
      { id: 'sfr.ios', family: 'sfr', familyName: 'SFR', platform: 'ios', platformName: 'iOS', main: false },
    ],
    summary: { clients: 4, identical: 0, withDifferences: 0, withProblems: 0, untested: 0 },
    weighted: { supported: 0, partial: 0, unsupported: 0 },
    features,
  }
}

const grid = feature('css-display-grid', [
  ['outlook.windows', '2016', false, 'no'],
  ['outlook.windows', '2019', true, 'partial', [1]],
  ['gmail.desktop-webmail', '2019-02', true, 'yes'],
  ['apple-mail.ios', '12.1', true, 'yes', [2]],
])
const hover = feature('css-pseudo-class-hover', [
  ['gmail.desktop-webmail', '2019-02', true, 'no'],
  ['apple-mail.ios', '12.1', true, 'yes'],
])

describe('platformGroup', () => {
  it('folds caniemail platforms into the user-facing groups', () => {
    expect(['windows', 'windows-mail', 'macos', 'desktop-app'].map(platformGroup)).toEqual(['desktop', 'desktop', 'desktop', 'desktop'])
    expect(['desktop-webmail', 'mobile-webmail', 'webmail', 'outlook-com'].map(platformGroup)).toEqual(['webmail', 'webmail', 'webmail', 'webmail'])
    expect(platformGroup('ios')).toBe('ios')
    expect(platformGroup('android')).toBe('android')
  })
})

describe('featureResults', () => {
  it('uses the latest version by default and the worst version when older versions are included', () => {
    const clients = report([grid]).clients
    const latest = featureResults(grid, clients, DEFAULT_COMPATIBILITY_FILTER)
    expect(latest.map((result) => result.support)).toEqual(['partial', 'yes', 'yes', 'unknown', 'unknown'])
    expect(latest[0]).toMatchObject({ versions: ['2019'], notes: [1] })
    const all = featureResults(grid, clients, { ...DEFAULT_COMPATIBILITY_FILTER, olderVersions: true })
    expect(all[0]).toMatchObject({ support: 'no', versions: ['2016'], notes: [] })
    expect(featureCounts(latest)).toEqual({ yes: 2, no: 0, partial: 1, unknown: 2 })
  })
})

describe('summarizeReport', () => {
  it('lets the worst known result decide each selected client and counts clients without data as untested', () => {
    const compatibility = report([grid, hover])
    expect(summarizeReport(compatibility, DEFAULT_COMPATIBILITY_FILTER)).toEqual({ clients: 4, identical: 1, withDifferences: 1, withProblems: 1, untested: 1 })
    const mobile = { scope: 'main' as const, platforms: ['ios', 'android'] as const, olderVersions: false }
    expect(selectedClients(compatibility, { ...mobile, platforms: [...mobile.platforms] }).map((client) => client.id)).toEqual(['apple-mail.ios', 'samsung-email.android'])
    expect(summarizeReport(compatibility, { ...mobile, platforms: [...mobile.platforms] })).toEqual({ clients: 2, identical: 1, withDifferences: 0, withProblems: 0, untested: 1 })
  })
})

describe('unscored features', () => {
  it('are excluded from the client summary and the weighted score', () => {
    const body = { ...feature('html-body', [['gmail.desktop-webmail', '2019-02', true, 'no']]), scored: false }
    const compatibility = report([body])
    expect(summarizeReport(compatibility, DEFAULT_COMPATIBILITY_FILTER)).toEqual({ clients: 4, identical: 0, withDifferences: 0, withProblems: 0, untested: 4 })
    expect(weightedScore(compatibility, DEFAULT_COMPATIBILITY_FILTER)).toEqual({ supported: 100, partial: 0, unsupported: 0 })
  })
})

describe('weighting', () => {
  it('weights shares by element use and lets the worst feature decide', () => {
    const compatibility = report([{ ...hover, occurrences: 2 }, grid])
    expect(featureWeight(compatibility.features[0]!, compatibility)).toBe(0.5)
    expect(featureWeight({ ...grid, occurrences: 9 }, compatibility)).toBe(1)
    expect(featureImpact({ yes: 1, no: 1, partial: 0, unknown: 2 }, 0.5)).toBe(25)
    // hover: 50% unsupported of known clients × 0.5; grid: 33% partial × 0.25.
    const score = weightedScore(compatibility, DEFAULT_COMPATIBILITY_FILTER)
    expect(score.unsupported).toBe(25)
    expect(score.partial).toBeCloseTo(100 / 12)
    expect(score.supported).toBeCloseTo(100 - 25 - 100 / 12)
  })
})

describe('sortFeatures', () => {
  it('puts scored features first, then the highest impact', () => {
    const body = { ...grid, slug: 'html-body', scored: false }
    const sorted = sortFeatures([
      { feature: body, counts: { yes: 0, no: 4, partial: 0, unknown: 0 }, impact: 90 },
      { feature: grid, counts: { yes: 2, no: 0, partial: 1, unknown: 1 }, impact: 8 },
      { feature: hover, counts: { yes: 1, no: 1, partial: 0, unknown: 2 }, impact: 12 },
    ])
    expect(sorted.map(({ feature }) => feature.slug)).toEqual(['css-pseudo-class-hover', 'css-display-grid', 'html-body'])
  })
})

describe('markdownSegments', () => {
  it('keeps inline code and http links and leaves other links and markup as text', () => {
    expect(markdownSegments('Use `display:flex` instead, see [docs](https://example.test/a) or [x](javascript:void).')).toEqual([
      { kind: 'text', text: 'Use ' },
      { kind: 'code', text: 'display:flex' },
      { kind: 'text', text: ' instead, see ' },
      { kind: 'link', text: 'docs', href: 'https://example.test/a' },
      { kind: 'text', text: ' or ' },
      { kind: 'text', text: 'x' },
      { kind: 'text', text: '.' },
    ])
    expect(markdownSegments('<b>plain</b>')).toEqual([{ kind: 'text', text: '<b>plain</b>' }])
  })
})

describe('parseCompatibilityFilter', () => {
  it('restores known platforms and falls back to the default for invalid storage', () => {
    expect(parseCompatibilityFilter('{"platforms":["ios","bogus"],"olderVersions":true}')).toEqual({ scope: 'main', platforms: ['ios'], olderVersions: true })
    expect(parseCompatibilityFilter('{"scope":"all"}')).toMatchObject({ scope: 'all' })
    expect(parseCompatibilityFilter('{"platforms":[]}')).toMatchObject({ platforms: ['desktop', 'webmail', 'ios', 'android'] })
    expect(parseCompatibilityFilter('not json')).toEqual(DEFAULT_COMPATIBILITY_FILTER)
    expect(parseCompatibilityFilter(null)).toEqual(DEFAULT_COMPATIBILITY_FILTER)
  })
})

describe('client tiers', () => {
  it('turns missing graceful features into differences and shows only main clients by default', () => {
    const gracefulHover = { ...hover, graceful: true }
    const compatibility = report([grid, gracefulHover])
    expect(summarizeReport(compatibility, DEFAULT_COMPATIBILITY_FILTER)).toEqual({ clients: 4, identical: 1, withDifferences: 2, withProblems: 0, untested: 1 })
    expect(summarizeReport(compatibility, { ...DEFAULT_COMPATIBILITY_FILTER, scope: 'all' }).clients).toBe(5)
  })

  it('places features as problems, differences, unscored or supported', () => {
    expect(featureSeverity(hover, { yes: 1, no: 1, partial: 0, unknown: 0 })).toBe('problem')
    expect(featureSeverity({ ...hover, graceful: true }, { yes: 1, no: 1, partial: 0, unknown: 0 })).toBe('difference')
    expect(featureSeverity(grid, { yes: 2, no: 0, partial: 1, unknown: 0 })).toBe('difference')
    expect(featureSeverity({ ...grid, scored: false }, { yes: 0, no: 3, partial: 0, unknown: 0 })).toBe('unscored')
    expect(featureSeverity(grid, { yes: 3, no: 0, partial: 0, unknown: 1 })).toBe('supported')
  })
})
