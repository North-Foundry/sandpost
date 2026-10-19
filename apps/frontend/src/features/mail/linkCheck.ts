import type { LinkCheckReport, LinkCheckResult, LinkOutcome } from '@/api/contracts'

/**
 * Longest link the server verifies; longer links never have results.
 */
const MAXIMUM_URL_LENGTH = 2048

/**
 * Outcomes from most to least concerning, the order used by lists and summaries.
 */
export const OUTCOMES_BY_CONCERN: readonly LinkOutcome[] = ['broken', 'server_error', 'failed', 'inconclusive', 'healthy']

/**
 * Status codes that refuse automated requests to pages that usually work in a browser.
 */
const ACCESS_REFUSALS = new Set([401, 403, 407, 429])

/**
 * Semantic color family used to mark an outcome in application UI, never in the email itself.
 */
export type LinkTone = 'success' | 'destructive' | 'warning' | 'muted'

/**
 * Background classes of the dot that marks each tone in application UI.
 */
export const TONE_DOTS: Record<LinkTone, string> = {
  success: 'bg-success',
  destructive: 'bg-destructive',
  warning: 'bg-warning',
  muted: 'bg-muted-foreground/40',
}

/**
 * Show a URL without its scheme, which every checked link shares.
 */
export function displayUrl(url: string): string {
  return url.replace(/^https?:\/\//, '')
}

/**
 * Text and color of one link's stored result, ready for a popover or list row.
 */
export interface LinkResultDescription {
  tone: LinkTone
  title: string
  status: string
  explanation: string
  destination: string | null
  checked: string
  checkedAbsolute: string
}

/**
 * Translate a message key with named parameters.
 */
type Translate = (key: string, parameters?: Record<string, unknown>) => string

/**
 * Normalize an `href` exactly like the server identifies links: absolute HTTP(S) URLs only, parsed
 * with the WHATWG URL rules and without their fragment.
 *
 * Relative references are rejected rather than resolved because captured mail has no base URL.
 */
export function normalizeLinkHref(href: string | null | undefined): string | null {
  const value = href?.trim()
  if (!value || value.length > MAXIMUM_URL_LENGTH) return null
  let url: URL
  try {
    url = new URL(value)
  } catch {
    return null
  }
  if ((url.protocol !== 'http:' && url.protocol !== 'https:') || !url.hostname) return null
  url.hash = ''
  return url.href.length <= MAXIMUM_URL_LENGTH ? url.href : null
}

/**
 * Index a report's results by normalized URL for hover lookups.
 */
export function resultsByUrl(report: LinkCheckReport | null): Map<string, LinkCheckResult> {
  return new Map((report?.links ?? []).map((link) => [link.url, link]))
}

/**
 * Return the color family of an outcome: green when the link works, red when the destination
 * answered with an error, and yellow when the check could not reach a verdict.
 */
export function outcomeTone(outcome: LinkOutcome): LinkTone {
  switch (outcome) {
    case 'healthy': return 'success'
    case 'broken':
    case 'server_error': return 'destructive'
    default: return 'warning'
  }
}

/**
 * Order results from most to least concerning, keeping message order within each outcome.
 */
export function sortByConcern(links: readonly LinkCheckResult[]): LinkCheckResult[] {
  return links
    .map((link, index) => ({ link, index }))
    .sort((first, second) =>
      OUTCOMES_BY_CONCERN.indexOf(first.link.outcome) - OUTCOMES_BY_CONCERN.indexOf(second.link.outcome)
      || first.index - second.index)
    .map(({ link }) => link)
}

/**
 * Return the outcome counts of a report that are not zero, most concerning last after healthy.
 *
 * The order mirrors how people read the summary: healthy first, then each kind of problem.
 */
export function summaryParts(report: LinkCheckReport): { outcome: LinkOutcome; count: number }[] {
  const counts: Record<LinkOutcome, number> = {
    healthy: report.summary.healthy,
    broken: report.summary.broken,
    server_error: report.summary.serverError,
    failed: report.summary.failed,
    inconclusive: report.summary.inconclusive,
  }
  return (['healthy', 'broken', 'server_error', 'failed', 'inconclusive'] as const)
    .filter((outcome) => counts[outcome] > 0)
    .map((outcome) => ({ outcome, count: counts[outcome] }))
}

/**
 * Return the message key of a short label for a result's status code or failure.
 */
export function statusLabel(result: LinkCheckResult, t: Translate): string {
  if (result.statusCode !== null && result.errorKind === null) {
    return result.statusText
      ? t('mail.links.status.http', { code: result.statusCode, text: result.statusText })
      : t('mail.links.status.httpCode', { code: result.statusCode })
  }
  return t(`mail.links.error.${result.errorKind ?? 'protocol'}`)
}

/**
 * Return the shortest status of a result: its final status code, or the failure label.
 */
export function shortStatus(result: LinkCheckResult, t: Translate): string {
  return result.statusCode !== null && result.errorKind === null ? String(result.statusCode) : statusLabel(result, t)
}

/**
 * Return a one-sentence explanation of what a result means for the link.
 */
export function explanation(result: LinkCheckResult, t: Translate): string {
  if (result.errorKind === 'blocked_address') return t('mail.links.explanation.blocked')
  if (result.errorKind === 'too_many_redirects' || result.errorKind === 'redirect_loop' || result.errorKind === 'invalid_redirect') {
    return t('mail.links.explanation.redirect')
  }
  if (result.errorKind === 'timeout') return t('mail.links.explanation.timeout')
  if (result.errorKind === 'name_resolution') return t('mail.links.explanation.nameResolution')
  if (result.errorKind === 'tls') return t('mail.links.explanation.tls')
  if (result.outcome === 'failed') return t('mail.links.explanation.failed')
  if (result.statusCode !== null && ACCESS_REFUSALS.has(result.statusCode)) return t('mail.links.explanation.refused')
  return t(`mail.links.explanation.${result.outcome}`)
}

/**
 * Describe where a link ended up when redirects were followed.
 */
export function destination(result: LinkCheckResult, t: Translate): string | null {
  if (!result.finalUrl || result.redirects === 0) return null
  return t(result.redirects === 1 ? 'mail.links.redirected_one' : 'mail.links.redirected_other', {
    count: result.redirects,
    url: result.finalUrl,
  })
}

/**
 * Pick the largest unit that keeps a relative time readable.
 */
export function relativeTimeParts(seconds: number, now: number): { value: number; unit: Intl.RelativeTimeFormatUnit } {
  const elapsed = Math.round(seconds - now / 1000)
  const magnitude = Math.abs(elapsed)
  if (magnitude < 45) return { value: 0, unit: 'second' }
  if (magnitude < 45 * 60) return { value: Math.round(elapsed / 60), unit: 'minute' }
  if (magnitude < 22 * 3600) return { value: Math.round(elapsed / 3600), unit: 'hour' }
  if (magnitude < 26 * 86400) return { value: Math.round(elapsed / 86400), unit: 'day' }
  if (magnitude < 320 * 86400) return { value: Math.round(elapsed / (30 * 86400)), unit: 'month' }
  return { value: Math.round(elapsed / (365 * 86400)), unit: 'year' }
}

/**
 * Format a Unix-seconds timestamp relative to now, such as "2 minutes ago" or "now".
 */
export function formatRelativeTime(seconds: number, now: number, locale: string): string {
  const { value, unit } = relativeTimeParts(seconds, now)
  return new Intl.RelativeTimeFormat(locale, { numeric: 'auto' }).format(value, unit)
}

/**
 * Describe one stored result for the hover popover and the detailed list.
 */
export function describeLinkResult(
  result: LinkCheckResult,
  t: Translate,
  now: number,
  locale: string,
  formatAbsolute: (timestamp: number) => string,
): LinkResultDescription {
  return {
    tone: outcomeTone(result.outcome),
    title: t(`mail.links.outcome.${result.outcome}`),
    status: statusLabel(result, t),
    explanation: explanation(result, t),
    destination: destination(result, t),
    checked: t('mail.links.checkedAt', { time: formatRelativeTime(result.checkedAt, now, locale) }),
    checkedAbsolute: formatAbsolute(result.checkedAt * 1000),
  }
}
