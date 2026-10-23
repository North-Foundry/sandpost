import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'
import { effectScope, nextTick, ref } from 'vue'
import type { OutboundDelivery, SandpostClient } from '@/api/contracts'
import { MESSAGE_DELIVERY_POLL_INTERVAL, useMessageDeliveries } from './useMessageDeliveries'

/**
 * Build a delivery in the requested state.
 */
function delivery(status: OutboundDelivery['status']): OutboundDelivery {
  return {
    id: `delivery-${status}`, sequence: 1, messageId: 'message-1', subject: 'Hi', scopeId: null, routingActionId: null,
    smtpOutboundId: null, smtpOutboundName: 'Pippo', actionType: 'relay', status, envelopeFrom: null, attemptCount: 1,
    lastError: null, createdAt: 0, lastAttemptAt: null, nextAttemptAt: null, deliveredAt: null, recipients: [],
  }
}

/**
 * Flush promise continuations and Vue watchers.
 */
async function flush(): Promise<void> {
  for (let index = 0; index < 5; index += 1) { await Promise.resolve(); await nextTick() }
}

describe('useMessageDeliveries', () => {
  let client: SandpostClient
  let forMessage: ReturnType<typeof vi.fn>

  beforeEach(() => {
    vi.useFakeTimers()
    forMessage = vi.fn(async () => [delivery('retrying')])
    client = { admin: { deliveries: { forMessage } } } as unknown as SandpostClient
  })

  afterEach(() => vi.useRealTimers())

  /**
   * Members and Viewers never call the Owner-only endpoint.
   */
  it('requests nothing unless enabled', async () => {
    const scope = effectScope()
    const state = scope.run(() => useMessageDeliveries(client, ref('message-1'), ref(false)))!
    await flush()
    expect(forMessage).not.toHaveBeenCalled()
    expect(state.deliveries.value).toEqual([])
    scope.stop()
  })

  /**
   * Owners load the open message's deliveries and poll while one is unfinished.
   */
  it('loads and polls unfinished deliveries for Owners', async () => {
    const scope = effectScope()
    const state = scope.run(() => useMessageDeliveries(client, ref('message-1'), ref(true)))!
    await flush()
    expect(forMessage).toHaveBeenCalledWith('message-1')
    expect(state.deliveries.value.map((item) => item.status)).toEqual(['retrying'])
    forMessage.mockResolvedValueOnce([delivery('delivered')])
    await vi.advanceTimersByTimeAsync(MESSAGE_DELIVERY_POLL_INTERVAL)
    await flush()
    expect(forMessage).toHaveBeenCalledTimes(2)
    expect(state.deliveries.value.map((item) => item.status)).toEqual(['delivered'])
    await vi.advanceTimersByTimeAsync(MESSAGE_DELIVERY_POLL_INTERVAL * 2)
    expect(forMessage).toHaveBeenCalledTimes(2)
    scope.stop()
  })

  /**
   * Failures stay silent, and switching messages discards the previous list.
   */
  it('stays quiet on failure and resets on message change', async () => {
    const messageId = ref<string | null>('message-1')
    const scope = effectScope()
    const state = scope.run(() => useMessageDeliveries(client, messageId, ref(true)))!
    await flush()
    forMessage.mockRejectedValueOnce(new Error('forbidden'))
    messageId.value = 'message-2'
    await flush()
    expect(state.deliveries.value).toEqual([])
    messageId.value = null
    await flush()
    expect(forMessage).toHaveBeenCalledTimes(2)
    scope.stop()
  })
})
