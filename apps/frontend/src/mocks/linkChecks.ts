import type { BulkLinkCheckResult, LinkCheckReport, LinkCheckResult, LinkOutcome, MessageDetail } from '@/api/contracts'
import { normalizeLinkHref } from '@/features/mail/linkCheck'

/**
 * How long a simulated check stays running, so progress feedback is visible in the mock.
 */
const MOCK_CHECK_DURATION = 1200

/**
 * Collect the distinct absolute links of a mock message in first-appearance order.
 */
function mockMessageLinks(detail: MessageDetail): string[] {
  const candidates = [
    ...[...detail.html.matchAll(/<a\s[^>]*href="([^"]*)"/gi)].map((match) => match[1]!.replaceAll('&amp;', '&')),
    ...[...detail.text.matchAll(/https?:\/\/[^\s<>"']+/gi)].map((match) => match[0]),
  ]
  return [...new Set(candidates.map(normalizeLinkHref).filter((url): url is string => url !== null))]
}

/**
 * Derive a deterministic synthetic verdict from words in the link's path or host.
 */
function mockResult(url: string, checkedAt: number): LinkCheckResult {
  /**
   * Build a result with the shared defaults of the simulated checker.
   */
  const result = (outcome: LinkOutcome, fields: Partial<LinkCheckResult>): LinkCheckResult => ({
    url, outcome, statusCode: null, statusText: null, finalUrl: null, redirects: 0, errorKind: null, errorMessage: null, checkedAt, ...fields,
  })
  const { hostname, pathname } = new URL(url)
  if (['localhost', '127.0.0.1', '[::1]'].includes(hostname)) {
    return result('inconclusive', { errorKind: 'blocked_address', errorMessage: `${hostname === 'localhost' ? '127.0.0.1' : hostname} is not a public address` })
  }
  if (pathname.includes('missing')) return result('broken', { statusCode: 404, statusText: 'Not Found' })
  if (pathname.includes('server-error')) return result('server_error', { statusCode: 503, statusText: 'Service Unavailable' })
  if (pathname.includes('timeout')) return result('failed', { errorKind: 'timeout', errorMessage: 'no response within 10 seconds' })
  if (pathname.includes('forbidden')) return result('inconclusive', { statusCode: 403, statusText: 'Forbidden' })
  if (pathname.includes('redirect')) {
    return result('healthy', { statusCode: 200, statusText: 'OK', redirects: 1, finalUrl: url.replace('/redirect', '') })
  }
  return result('healthy', { statusCode: 200, statusText: 'OK' })
}

/**
 * Count results by outcome for the report summary.
 */
function summarize(links: LinkCheckResult[]): LinkCheckReport['summary'] {
  const count = (outcome: LinkOutcome) => links.filter((link) => link.outcome === outcome).length
  return {
    total: links.length,
    healthy: count('healthy'),
    broken: count('broken'),
    serverError: count('server_error'),
    failed: count('failed'),
    inconclusive: count('inconclusive'),
  }
}

/**
 * Simulated, shared link checks for the mock client; results survive for the client lifetime.
 */
export function createMockLinkChecks(getMessage: (id: string) => MessageDetail | undefined) {
  const stored = new Map<string, { checkedAt: number; links: LinkCheckResult[] }>()
  const running = new Map<string, number>()

  /**
   * Return the current state of a known message.
   */
  function report(id: string): LinkCheckReport {
    const check = stored.get(id)
    const links = check ? structuredClone(check.links) : []
    const detail = getMessage(id)
    return {
      messageId: id,
      running: running.has(id),
      startedAt: running.get(id) ?? null,
      checkedAt: check?.checkedAt ?? null,
      urls: detail ? mockMessageLinks(detail) : [],
      omittedLinks: 0,
      summary: summarize(links),
      links,
    }
  }

  /**
   * Start a simulated run unless one is already running, returning whether it started.
   */
  function start(id: string): boolean {
    const detail = getMessage(id)
    if (!detail) throw new Error(`Mock message not found: ${id}`)
    if (running.has(id)) return false
    running.set(id, Math.floor(Date.now() / 1000))
    setTimeout(() => {
      const checkedAt = Math.floor(Date.now() / 1000)
      stored.set(id, { checkedAt, links: mockMessageLinks(detail).map((url) => mockResult(url, checkedAt)) })
      running.delete(id)
    }, MOCK_CHECK_DURATION)
    return true
  }

  return {
    /**
     * Read a message's state without starting a check.
     */
    get(id: string): LinkCheckReport {
      if (!getMessage(id)) throw new Error(`Mock message not found: ${id}`)
      return report(id)
    },

    /**
     * Start or join a check of one message.
     */
    start(id: string): LinkCheckReport {
      start(id)
      return report(id)
    },

    /**
     * Start checks of several messages, skipping unknown and duplicate identifiers.
     */
    startMany(identifiers: string[]): BulkLinkCheckResult {
      if (!identifiers.length || identifiers.length > 256) throw new Error('select between 1 and 256 messages')
      const result: BulkLinkCheckResult = { started: 0, alreadyRunning: 0, skipped: 0 }
      for (const id of new Set(identifiers)) {
        if (!getMessage(id)) { result.skipped += 1; continue }
        if (start(id)) result.started += 1
        else result.alreadyRunning += 1
      }
      result.skipped += identifiers.length - new Set(identifiers).size
      return result
    },
  }
}
