import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'
import { effectScope, nextTick, ref } from 'vue'
import type { LinkCheckReport, SandpostClient } from '@/api/contracts'
import { LINK_CHECK_POLL_INTERVAL, useLinkCheck } from './useLinkCheck'

vi.mock('vue-i18n', () => ({
  /**
   * Return message keys so error fallbacks are observable without a component instance.
   */
  useI18n: () => ({ t: (key: string) => key }),
}))

/**
 * Build a report for a message with optional overrides.
 */
function report(fields: Partial<LinkCheckReport> = {}): LinkCheckReport {
  return {
    messageId: 'message-1', running: false, startedAt: null, checkedAt: null, urls: ['https://example.test/'], omittedLinks: 0,
    summary: { total: 0, healthy: 0, broken: 0, serverError: 0, failed: 0, inconclusive: 0 },
    links: [], ...fields,
  }
}

const previous = report({
  checkedAt: 100,
  summary: { total: 1, healthy: 0, broken: 1, serverError: 0, failed: 0, inconclusive: 0 },
  links: [{ url: 'https://example.test/', outcome: 'broken', statusCode: 404, statusText: 'Not Found', finalUrl: null, redirects: 0, errorKind: null, errorMessage: null, checkedAt: 100 }],
})

/**
 * Flush promise continuations and Vue watchers.
 */
async function flush(): Promise<void> {
  for (let index = 0; index < 5; index += 1) { await Promise.resolve(); await nextTick() }
}

describe('useLinkCheck', () => {
  let client: SandpostClient

  beforeEach(() => {
    vi.useFakeTimers()
    client = {
      messages: {
        linkCheck: vi.fn(async () => previous),
        startLinkCheck: vi.fn(async () => ({ ...previous, running: true, startedAt: 200 })),
      },
    } as unknown as SandpostClient
  })

  afterEach(() => vi.useRealTimers())

  it('reads stored results on open without starting a check', async () => {
    const scope = effectScope()
    const state = scope.run(() => useLinkCheck(client, ref('message-1')))!
    await flush()
    expect(client.messages.linkCheck).toHaveBeenCalledExactlyOnceWith('message-1')
    expect(client.messages.startLinkCheck).not.toHaveBeenCalled()
    expect(state.report.value).toEqual(previous)
    expect(state.results.value.get('https://example.test/')?.statusCode).toBe(404)
    await vi.advanceTimersByTimeAsync(LINK_CHECK_POLL_INTERVAL * 3)
    expect(client.messages.linkCheck).toHaveBeenCalledTimes(1)
    scope.stop()
  })

  it('keeps previous results visible while a recheck runs, then shows the replacement', async () => {
    const replacement = report({ checkedAt: 300, summary: { total: 1, healthy: 1, broken: 0, serverError: 0, failed: 0, inconclusive: 0 } })
    vi.mocked(client.messages.linkCheck)
      .mockResolvedValueOnce(previous)
      .mockResolvedValueOnce({ ...previous, running: true, startedAt: 200 })
      .mockResolvedValueOnce(replacement)
    const scope = effectScope()
    const state = scope.run(() => useLinkCheck(client, ref('message-1')))!
    await flush()
    await state.start()
    expect(state.running.value).toBe(true)
    expect(state.report.value?.links).toEqual(previous.links)
    await vi.advanceTimersByTimeAsync(LINK_CHECK_POLL_INTERVAL)
    expect(state.report.value?.links).toEqual(previous.links)
    await vi.advanceTimersByTimeAsync(LINK_CHECK_POLL_INTERVAL)
    expect(state.report.value).toEqual(replacement)
    expect(state.running.value).toBe(false)
    await vi.advanceTimersByTimeAsync(LINK_CHECK_POLL_INTERVAL * 3)
    expect(client.messages.linkCheck).toHaveBeenCalledTimes(3)
    scope.stop()
  })

  it('follows a check another user started and ignores responses for a message no longer open', async () => {
    let resolveFirst!: (value: LinkCheckReport) => void
    vi.mocked(client.messages.linkCheck)
      .mockImplementationOnce(() => new Promise((resolve) => { resolveFirst = resolve }))
      .mockResolvedValueOnce(report({ messageId: 'message-2', running: true }))
      .mockResolvedValueOnce(report({ messageId: 'message-2', checkedAt: 400 }))
    const messageId = ref('message-1')
    const scope = effectScope()
    const state = scope.run(() => useLinkCheck(client, messageId))!
    await flush()
    messageId.value = 'message-2'
    await flush()
    resolveFirst(previous)
    await flush()
    expect(state.report.value?.messageId).toBe('message-2')
    expect(state.running.value).toBe(true)
    await vi.advanceTimersByTimeAsync(LINK_CHECK_POLL_INTERVAL)
    expect(state.report.value).toMatchObject({ messageId: 'message-2', checkedAt: 400, running: false })
    scope.stop()
  })

  it('reports refused starts and stops polling when disposed', async () => {
    vi.mocked(client.messages.startLinkCheck).mockRejectedValueOnce(new Error('forbidden'))
    const scope = effectScope()
    const state = scope.run(() => useLinkCheck(client, ref('message-1')))!
    await flush()
    await state.start()
    expect(state.error.value).toBe('Forbidden')
    expect(state.report.value).toEqual(previous)
    vi.mocked(client.messages.linkCheck).mockResolvedValue({ ...previous, running: true })
    await state.refresh()
    scope.stop()
    await vi.advanceTimersByTimeAsync(LINK_CHECK_POLL_INTERVAL * 5)
    expect(client.messages.linkCheck).toHaveBeenCalledTimes(2)
  })
})
