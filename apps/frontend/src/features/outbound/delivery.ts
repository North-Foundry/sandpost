/**
 * Presentation helpers for outbound deliveries.
 */
import type {
  OutboundAttemptOutcome,
  OutboundDelivery,
  OutboundDeliveryStatus,
  OutboundRecipientStatus,
} from '@/api/contracts'
import type { StatusTone } from '@/features/personas/format'

/**
 * Every delivery status, in lifecycle order, for filters.
 */
export const DELIVERY_STATUSES: readonly OutboundDeliveryStatus[] = ['pending', 'processing', 'retrying', 'delivered', 'failed']

/**
 * Color a delivery status: delivered is green, failed red, retrying a warning and queued work in progress.
 */
export function deliveryTone(status: OutboundDeliveryStatus): StatusTone {
  switch (status) {
    case 'delivered': return 'success'
    case 'failed': return 'destructive'
    case 'retrying': return 'warning'
    default: return 'progress'
  }
}

/**
 * Color one recipient's state.
 */
export function recipientTone(status: OutboundRecipientStatus): StatusTone {
  switch (status) {
    case 'delivered': return 'success'
    case 'failed': return 'destructive'
    default: return 'progress'
  }
}

/**
 * Color an attempt outcome; partial and deferred attempts leave work for a later attempt.
 */
export function attemptTone(outcome: OutboundAttemptOutcome): StatusTone {
  switch (outcome) {
    case 'delivered': return 'success'
    case 'failed': return 'destructive'
    case 'interrupted': return 'muted'
    default: return 'warning'
  }
}

/**
 * Report whether a delivery can still change, so its view should refresh.
 */
export function isDeliveryUnfinished(status: OutboundDeliveryStatus): boolean {
  return status === 'pending' || status === 'processing' || status === 'retrying'
}

/**
 * Only failed deliveries can be scheduled again.
 */
export function canRetryDelivery(delivery: Pick<OutboundDelivery, 'status'>): boolean {
  return delivery.status === 'failed'
}

/**
 * Count recipients by state.
 */
export function recipientCounts(delivery: Pick<OutboundDelivery, 'recipients'>): Record<OutboundRecipientStatus, number> {
  const counts: Record<OutboundRecipientStatus, number> = { pending: 0, delivered: 0, failed: 0 }
  for (const recipient of delivery.recipients) counts[recipient.status] += 1
  return counts
}

/**
 * Summarize the worst state across deliveries, for a compact status line: any failure wins, then
 * retrying, then unfinished work; otherwise everything was delivered.
 */
export function overallDeliveryStatus(deliveries: readonly Pick<OutboundDelivery, 'status'>[]): OutboundDeliveryStatus | null {
  if (!deliveries.length) return null
  for (const status of ['failed', 'retrying', 'processing', 'pending'] as const) {
    if (deliveries.some((delivery) => delivery.status === status)) return status
  }
  return 'delivered'
}

/**
 * Format an error for display as `kind label · [code] message`, leaving the server text unchanged.
 * The code is not repeated when the server text already starts with it.
 */
export function outboundErrorText(error: { kind: string; message: string; code: number | null }, kindLabel: string): string {
  const code = error.code === null || error.message.startsWith(String(error.code)) ? '' : `${error.code} `
  return error.message ? `${kindLabel} · ${code}${error.message}` : `${kindLabel}${code ? ` · ${code.trim()}` : ''}`
}
