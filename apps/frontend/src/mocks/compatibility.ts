import type {
  ClientSupport,
  CompatibilityFeature,
  EmailClient,
  HtmlCompatibilityReport,
  SupportLevel,
} from '@/api/contracts'
import {
  DEFAULT_COMPATIBILITY_FILTER,
  featureCounts,
  featureImpact,
  featureResults,
  summarizeReport,
  weightedScore,
} from '@/features/mail/htmlCompatibility'

/**
 * Elements inside the synthetic bodies' `<body>`, used to weight features.
 */
const ELEMENT_COUNT = 9

/**
 * Structural features excluded from scores, as in the Rust check.
 */
const UNSCORED_SLUGS = ['html-body', 'html-doctype', 'html-link', 'html-style', 'html-comments', 'css-comments']

/**
 * Representative clients of the caniemail data, as `[family, family name, platform, platform name]`.
 */
const CLIENTS: [string, string, string, string][] = [
  ['gmail', 'Gmail', 'desktop-webmail', 'Desktop Webmail'],
  ['gmail', 'Gmail', 'ios', 'iOS'],
  ['gmail', 'Gmail', 'android', 'Android'],
  ['outlook', 'Outlook', 'windows', 'Windows'],
  ['outlook', 'Outlook', 'windows-mail', 'Windows Mail'],
  ['outlook', 'Outlook', 'macos', 'macOS'],
  ['outlook', 'Outlook', 'outlook-com', 'Outlook.com'],
  ['outlook', 'Outlook', 'ios', 'iOS'],
  ['outlook', 'Outlook', 'android', 'Android'],
  ['yahoo', 'Yahoo! Mail', 'desktop-webmail', 'Desktop Webmail'],
  ['apple-mail', 'Apple Mail', 'macos', 'macOS'],
  ['apple-mail', 'Apple Mail', 'ios', 'iOS'],
  ['thunderbird', 'Mozilla Thunderbird', 'windows', 'Windows'],
  ['samsung-email', 'Samsung Email', 'android', 'Android'],
  ['protonmail', 'ProtonMail', 'desktop-webmail', 'Desktop Webmail'],
  ['fastmail', 'Fastmail', 'desktop-webmail', 'Desktop Webmail'],
]

/**
 * A fixture feature: every client supports it unless listed in `exceptions` as `[level, ...notes]`.
 */
interface FixtureFeature {
  slug: string
  title: string
  category: string
  description: string
  notes?: Record<string, string>
  occurrences: number
  examples: string[]
  graceful?: boolean
  exceptions?: Record<string, [SupportLevel, ...number[]]>
}

/**
 * Client families shown by default, as in the Rust check.
 */
const MAIN_FAMILIES = ['apple-mail', 'gmail', 'outlook', 'yahoo', 'samsung-email']

/**
 * Features of the synthetic HTML bodies, with support resembling the published caniemail results.
 */
const FEATURES: FixtureFeature[] = [
  {
    slug: 'css-sytem-ui',
    title: 'system-ui, ui-serif, ui-sans-serif, ui-rounded, ui-monospace',
    category: 'css',
    description: 'Generic font families that map to the platform\'s user interface fonts.',
    notes: { '1': 'Falls back to the client\'s default font.' },
    occurrences: 1,
    examples: ['font: 14px/1.65 system-ui'],
    exceptions: {
      'gmail.desktop-webmail': ['no'],
      'gmail.ios': ['no'],
      'gmail.android': ['no'],
      'outlook.windows': ['no'],
      'outlook.windows-mail': ['no'],
      'outlook.outlook-com': ['partial', 1],
      'yahoo.desktop-webmail': ['no'],
      'samsung-email.android': ['partial', 1],
      'protonmail.desktop-webmail': ['unknown'],
    },
  },
  {
    slug: 'css-max-width',
    title: 'max-width',
    category: 'css',
    description: 'Sets the maximum width of an element.',
    occurrences: 1,
    examples: ['max-width: 580px'],
    exceptions: { 'outlook.windows': ['no'], 'outlook.windows-mail': ['no'] },
  },
  {
    slug: 'html-semantics',
    title: 'HTML5 semantics',
    category: 'html',
    description: 'Support for `<article>`, `<footer>`, `<header>` and other HTML5 sectioning elements.',
    notes: { '1': 'Elements are rendered but [styles applied to them](https://www.caniemail.com/) are ignored.' },
    occurrences: 2,
    examples: ['<article>', '<footer>'],
    exceptions: { 'outlook.windows': ['partial', 1], 'gmail.android': ['partial', 1] },
  },
  {
    slug: 'html-style',
    title: '<style> element',
    category: 'html',
    description: 'Support for embedded stylesheets in the `<head>`.',
    notes: { '1': 'Not supported in non-Google accounts.' },
    occurrences: 1,
    examples: ['<style>'],
    exceptions: { 'gmail.android': ['partial', 1], 'gmail.ios': ['partial', 1] },
  },
  {
    slug: 'css-pseudo-class-hover',
    title: ':hover',
    category: 'css',
    description: 'Styles applied while the pointer is over an element.',
    occurrences: 1,
    examples: ['a:hover'],
    graceful: true,
    exceptions: {
      'gmail.ios': ['no'],
      'gmail.android': ['no'],
      'outlook.windows': ['no'],
      'outlook.ios': ['no'],
      'outlook.android': ['no'],
      'samsung-email.android': ['no'],
    },
  },
  {
    slug: 'css-border',
    title: 'border',
    category: 'css',
    description: 'The `border` shorthand and its per-side variants.',
    occurrences: 1,
    examples: ['border-top: 1px solid #e4e4e7'],
  },
  {
    slug: 'css-unit-px',
    title: 'px unit',
    category: 'css',
    description: 'Absolute lengths in CSS pixels.',
    occurrences: 6,
    examples: ['padding: 28px', 'max-width: 580px', 'font-size: 22px'],
  },
  {
    slug: 'html-h1-h6',
    title: '<h1> to <h6> elements',
    category: 'html',
    description: 'Headings.',
    occurrences: 1,
    examples: ['<h1>'],
  },
  {
    slug: 'css-selector-type',
    title: 'Type selector',
    category: 'css',
    description: 'Selecting elements by tag name, such as `body` or `p`.',
    occurrences: 7,
    examples: ['body', 'article', 'h1'],
  },
]

/**
 * Build the client support results of a fixture feature.
 */
function supportResults(feature: FixtureFeature): ClientSupport[] {
  return CLIENTS.map(([family, , platform]) => {
    const client = `${family}.${platform}`
    const [support, ...notes] = feature.exceptions?.[client] ?? ['yes']
    return { client, version: '2024-04', latest: true, support, notes }
  })
}

/**
 * Return a synthetic compatibility report; bodies without HTML report the HTML as absent.
 */
export function mockCompatibilityReport(hasHtml: boolean): HtmlCompatibilityReport {
  const data = { source: 'caniemail.com', url: 'https://www.caniemail.com', updated: '2026-09-16 13:05:00 +0000', license: 'MIT' }
  if (!hasHtml) {
    return {
      htmlPresent: false, externalStylesheets: 0, elementCount: 0, data, clients: [],
      summary: { clients: 0, identical: 0, withDifferences: 0, withProblems: 0, untested: 0 },
      weighted: { supported: 0, partial: 0, unsupported: 0 }, features: [],
    }
  }
  const clients: EmailClient[] = CLIENTS.map(([family, familyName, platform, platformName]) => ({ id: `${family}.${platform}`, family, familyName, platform, platformName, main: MAIN_FAMILIES.includes(family) }))
  const features: CompatibilityFeature[] = FEATURES.map((feature) => ({
    slug: feature.slug,
    title: feature.title,
    category: feature.category,
    url: `https://www.caniemail.com/features/${feature.slug}/`,
    description: feature.description,
    notes: feature.notes ?? {},
    occurrences: feature.occurrences,
    examples: feature.examples,
    selectorUnverified: false,
    scored: !UNSCORED_SLUGS.includes(feature.slug),
    graceful: feature.graceful ?? false,
    impact: 0,
    support: supportResults(feature),
  }))
  for (const feature of features) {
    const weight = Math.min(feature.occurrences / ELEMENT_COUNT, 1)
    feature.impact = featureImpact(featureCounts(featureResults(feature, clients, DEFAULT_COMPATIBILITY_FILTER)), weight)
  }
  const report: HtmlCompatibilityReport = {
    htmlPresent: true, externalStylesheets: 0, elementCount: ELEMENT_COUNT, data, clients, features,
    summary: { clients: 0, identical: 0, withDifferences: 0, withProblems: 0, untested: 0 },
    weighted: { supported: 0, partial: 0, unsupported: 0 },
  }
  report.summary = summarizeReport(report, DEFAULT_COMPATIBILITY_FILTER)
  report.weighted = weightedScore(report, DEFAULT_COMPATIBILITY_FILTER)
  return report
}
