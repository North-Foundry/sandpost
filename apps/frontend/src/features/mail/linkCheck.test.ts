import { describe, expect, it } from 'vitest'
import type { LinkCheckReport, LinkCheckResult } from '@/api/contracts'
import { i18n } from '@/i18n'
import {
  describeLinkResult,
  displayUrl,
  formatRelativeTime,
  normalizeLinkHref,
  outcomeTone,
  shortStatus,
  relativeTimeParts,
  resultsByUrl,
  sortByConcern,
  summaryParts,
} from './linkCheck'

/**
 * Build a stored result with healthy defaults and the supplied overrides.
 */
function result(fields: Partial<LinkCheckResult>): LinkCheckResult {
  return {
    url: 'https://example.test/', outcome: 'healthy', statusCode: 200, statusText: 'OK', finalUrl: null, redirects: 0,
    errorKind: null, errorMessage: null, checkedAt: 1_700_000_000, ...fields,
  }
}

/**
 * Translate with the English catalog.
 */
const t = (key: string, parameters?: Record<string, unknown>) => {
  i18n.global.locale.value = 'en'
  return i18n.global.t(key, parameters ?? {})
}

describe('link check presentation', () => {
  it('normalizes hrefs exactly like the server identifies links', () => {
    expect(normalizeLinkHref('  HTTPS://Example.TEST  ')).toBe('https://example.test/')
    expect(normalizeLinkHref('https://example.test/a b?x=1#section')).toBe('https://example.test/a%20b?x=1')
    expect(normalizeLinkHref('http://example.test:80/path#')).toBe('http://example.test/path')
    for (const rejected of ['/relative', 'mailto:someone@example.test', 'tel:+15551234', 'javascript:alert(1)', 'ftp://example.test/file', 'https://', '', null, undefined]) {
      expect(normalizeLinkHref(rejected)).toBeNull()
    }
    expect(normalizeLinkHref(`https://example.test/${'a'.repeat(3000)}`)).toBeNull()
  })

  it('indexes results by URL and orders them from most to least concerning', () => {
    const links = [
      result({ url: 'https://a.test/' }),
      result({ url: 'https://b.test/', outcome: 'inconclusive' }),
      result({ url: 'https://c.test/', outcome: 'broken', statusCode: 404 }),
      result({ url: 'https://d.test/', outcome: 'failed', statusCode: null, errorKind: 'timeout' }),
      result({ url: 'https://e.test/', outcome: 'broken', statusCode: 410 }),
    ]
    expect(sortByConcern(links).map((link) => link.url)).toEqual([
      'https://c.test/', 'https://e.test/', 'https://d.test/', 'https://b.test/', 'https://a.test/',
    ])
    const report = { links } as LinkCheckReport
    expect(resultsByUrl(report).get('https://d.test/')?.errorKind).toBe('timeout')
    expect(resultsByUrl(null).size).toBe(0)
  })

  it('lists only non-zero counts, healthy first', () => {
    const report = {
      summary: { total: 7, healthy: 4, broken: 2, serverError: 0, failed: 1, inconclusive: 0 },
    } as LinkCheckReport
    expect(summaryParts(report)).toEqual([
      { outcome: 'healthy', count: 4 }, { outcome: 'broken', count: 2 }, { outcome: 'failed', count: 1 },
    ])
  })

  it('picks readable relative time units', () => {
    const now = 1_700_000_000_000
    expect(relativeTimeParts(1_700_000_000 - 10, now)).toEqual({ value: 0, unit: 'second' })
    expect(relativeTimeParts(1_700_000_000 - 120, now)).toEqual({ value: -2, unit: 'minute' })
    expect(relativeTimeParts(1_700_000_000 - 3 * 3600, now)).toEqual({ value: -3, unit: 'hour' })
    expect(relativeTimeParts(1_700_000_000 - 2 * 86400, now)).toEqual({ value: -2, unit: 'day' })
    expect(formatRelativeTime(1_700_000_000 - 120, now, 'en')).toBe('2 minutes ago')
    expect(formatRelativeTime(1_700_000_000, now, 'en')).toBe('now')
  })

  it('describes broken, redirected, refused and blocked links in plain language', () => {
    const now = 1_700_000_120_000
    const absolute = (timestamp: number) => `at ${timestamp}`
    expect(describeLinkResult(result({ outcome: 'broken', statusCode: 404, statusText: 'Not Found' }), t, now, 'en', absolute)).toEqual({
      tone: 'destructive',
      title: 'Broken link',
      status: 'HTTP 404 — Not Found',
      explanation: 'The destination returned an HTTP error.',
      destination: null,
      checked: 'Checked 2 minutes ago',
      checkedAbsolute: 'at 1700000000000',
    })
    expect(describeLinkResult(result({ finalUrl: 'https://www.example.test/', redirects: 2 }), t, now, 'en', absolute)).toMatchObject({
      tone: 'success', status: 'HTTP 200 — OK', destination: 'Redirected 2 times to https://www.example.test/',
    })
    expect(describeLinkResult(result({ outcome: 'inconclusive', statusCode: 403, statusText: 'Forbidden' }), t, now, 'en', absolute)).toMatchObject({
      tone: 'warning', title: 'Inconclusive', explanation: expect.stringContaining('refused the automated check'),
    })
    expect(describeLinkResult(result({ outcome: 'inconclusive', statusCode: null, statusText: null, errorKind: 'blocked_address' }), t, now, 'en', absolute)).toMatchObject({
      status: 'Private address', explanation: expect.stringContaining('private or local address'),
    })
    expect(describeLinkResult(result({ outcome: 'failed', statusCode: 302, statusText: 'Found', errorKind: 'redirect_loop', redirects: 1, finalUrl: 'https://example.test/' }), t, now, 'en', absolute)).toMatchObject({
      tone: 'warning', title: 'Check failed', status: 'Redirect loop',
    })
  })

  it('colors verdicts green, red or yellow and keeps list statuses short', () => {
    expect(['healthy', 'broken', 'server_error', 'failed', 'inconclusive'].map((outcome) => outcomeTone(outcome as LinkCheckResult['outcome'])))
      .toEqual(['success', 'destructive', 'destructive', 'warning', 'warning'])
    expect(shortStatus(result({ outcome: 'broken', statusCode: 404, statusText: 'Not Found' }), t)).toBe('404')
    expect(shortStatus(result({ outcome: 'failed', statusCode: null, statusText: null, errorKind: 'timeout' }), t)).toBe('Timed out')
    expect(shortStatus(result({ outcome: 'failed', statusCode: 302, statusText: 'Found', errorKind: 'redirect_loop' }), t)).toBe('Redirect loop')
    expect(displayUrl('https://example.test/a?b=1')).toBe('example.test/a?b=1')
  })
})
