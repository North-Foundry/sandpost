import { describe, expect, it } from 'vitest'
import type { OutboundDeliveryRecipient } from '@/api/contracts'
import {
  attemptTone,
  canRetryDelivery,
  deliveryTone,
  isDeliveryUnfinished,
  outboundErrorText,
  overallDeliveryStatus,
  recipientCounts,
  recipientTone,
} from './delivery'

/**
 * Build a recipient in the requested state.
 */
function recipient(status: OutboundDeliveryRecipient['status']): OutboundDeliveryRecipient {
  return { address: `${status}@example.com`, status, error: null, response: null, deliveredAt: null }
}

describe('outbound delivery presentation', () => {
  /**
   * Delivered is green, failed red, retrying a warning and queued work in progress.
   */
  it('colors statuses and outcomes', () => {
    expect(deliveryTone('delivered')).toBe('success')
    expect(deliveryTone('failed')).toBe('destructive')
    expect(deliveryTone('retrying')).toBe('warning')
    expect(deliveryTone('pending')).toBe('progress')
    expect(recipientTone('failed')).toBe('destructive')
    expect(attemptTone('partial')).toBe('warning')
    expect(attemptTone('interrupted')).toBe('muted')
  })

  /**
   * Only failed deliveries can be retried, and only unfinished ones refresh.
   */
  it('decides retry and refresh', () => {
    expect(canRetryDelivery({ status: 'failed' })).toBe(true)
    expect(canRetryDelivery({ status: 'retrying' })).toBe(false)
    expect(isDeliveryUnfinished('processing')).toBe(true)
    expect(isDeliveryUnfinished('delivered')).toBe(false)
  })

  /**
   * Count recipients by state and summarize the worst status across deliveries.
   */
  it('summarizes recipients and deliveries', () => {
    expect(recipientCounts({ recipients: [recipient('delivered'), recipient('failed'), recipient('delivered')] }))
      .toEqual({ pending: 0, delivered: 2, failed: 1 })
    expect(overallDeliveryStatus([])).toBeNull()
    expect(overallDeliveryStatus([{ status: 'delivered' }, { status: 'pending' }])).toBe('pending')
    expect(overallDeliveryStatus([{ status: 'retrying' }, { status: 'failed' }])).toBe('failed')
    expect(overallDeliveryStatus([{ status: 'delivered' }])).toBe('delivered')
  })

  /**
   * Keep the server text verbatim after the translated category and the SMTP code.
   */
  it('formats outbound errors', () => {
    expect(outboundErrorText({ kind: 'authentication', message: '5.7.8 bad credentials', code: 535 }, 'Authentication failed'))
      .toBe('Authentication failed · 535 5.7.8 bad credentials')
    expect(outboundErrorText({ kind: 'connection', message: 'refused', code: null }, 'Connection failed')).toBe('Connection failed · refused')
    expect(outboundErrorText({ kind: 'permanent', message: '', code: 550 }, 'Permanent failure')).toBe('Permanent failure · 550')
    expect(outboundErrorText({ kind: 'temporary', message: '451 4.3.0 try later', code: 451 }, 'Temporary failure')).toBe('Temporary failure · 451 4.3.0 try later')
  })
})
