import { describe, expect, it } from 'vitest'
import type { SmtpOutboundInput } from './contracts'
import type { WireOutboundDeliveryDetail, WireSmtpOutbound } from './wire'
import {
  normalizeConnectionTest,
  normalizeOutboundDeliveryDetail,
  normalizeOutboundPolicy,
  normalizeRoutingAction,
  normalizeSmtpOutbound,
  outboundDeliveryListQuery,
  outboundDeliveryListWire,
  routingActionWire,
  smtpOutboundTestWire,
  smtpOutboundWire,
} from './outbound'

const wireOutbound: WireSmtpOutbound = {
  id: 'outbound-1', name: 'Pippo', host: 'smtp.example.com', port: 587, security: 'starttls', auth_type: 'plain',
  username: 'user', password_configured: true, enabled: true, override_from: null, envelope_from: 'bounces@example.com',
  connect_timeout: null, smtp_timeout: 30, tls_verify: true, helo_name: null, max_connections: null,
  allowed_recipients: [], routing_action_count: 2, created_at: 1_700_000_000, updated_at: 1_700_000_100,
}

const input: SmtpOutboundInput = {
  name: ' Pippo ', host: ' smtp.example.com ', port: 587, security: 'starttls', authenticationType: 'plain', username: ' user ',
  overrideFrom: ' ', envelopeFrom: null, connectTimeoutSeconds: null, smtpTimeoutSeconds: 30, tlsVerify: false, heloName: '',
  maxConnections: 4, allowedRecipients: [],
}

describe('SMTP Outbound DTO helpers', () => {
  /**
   * Convert timestamps to milliseconds, keep the empty allowlist and never copy a stray password.
   */
  it('normalizes outbounds without exposing passwords', () => {
    const outbound = normalizeSmtpOutbound({ ...wireOutbound, password: 'leaked' } as WireSmtpOutbound)
    expect(outbound).toEqual({
      id: 'outbound-1', name: 'Pippo', host: 'smtp.example.com', port: 587, security: 'starttls', authenticationType: 'plain',
      username: 'user', passwordConfigured: true, enabled: true, overrideFrom: null, envelopeFrom: 'bounces@example.com',
      connectTimeoutSeconds: null, smtpTimeoutSeconds: 30, tlsVerify: true, heloName: null, maxConnections: null,
      allowedRecipients: [], routingActionCount: 2, createdAt: 1_700_000_000_000, updatedAt: 1_700_000_100_000,
    })
    expect(JSON.stringify(outbound)).not.toContain('leaked')
    expect(normalizeSmtpOutbound({ ...wireOutbound, security: 'ssl' as never, auth_type: 'cram' as never }))
      .toMatchObject({ security: 'none', authenticationType: 'none' })
  })

  /**
   * Trim text, send blanks as null, keep the empty allowlist and leave out an unchanged password and state.
   */
  it('encodes outbound input and omits a blank password and unset enabled state', () => {
    expect(smtpOutboundWire(input)).toEqual({
      name: 'Pippo', host: 'smtp.example.com', port: 587, security: 'starttls', auth_type: 'plain', username: 'user',
      override_from: null, envelope_from: null, connect_timeout: null, smtp_timeout: 30, tls_verify: false, helo_name: null,
      max_connections: 4, allowed_recipients: [],
    })
    expect(smtpOutboundWire({ ...input, password: '', enabled: false })).not.toHaveProperty('password')
    expect(smtpOutboundWire({ ...input, password: '', enabled: false }).enabled).toBe(false)
    expect(smtpOutboundWire({ ...input, password: 's3cret' }).password).toBe('s3cret')
  })

  /**
   * Without authentication neither a username nor a password is sent.
   */
  it('drops credentials when the outbound does not authenticate', () => {
    const wire = smtpOutboundWire({ ...input, authenticationType: 'none', password: 's3cret' })
    expect(wire.username).toBeNull()
    expect(wire).not.toHaveProperty('password')
  })

  /**
   * Draft tests carry the edited outbound identifier only when there is one.
   */
  it('adds the outbound identifier to draft tests', () => {
    expect(smtpOutboundTestWire(input, 'outbound-1').id).toBe('outbound-1')
    expect(smtpOutboundTestWire(input)).not.toHaveProperty('id')
    expect(smtpOutboundTestWire(input, null)).not.toHaveProperty('id')
  })

  /**
   * Convert the policy, connection tests and unknown error kinds conservatively.
   */
  it('normalizes the policy and connection test results', () => {
    expect(normalizeOutboundPolicy({ allow_internal_addresses: false, maximum_attempts: 8, retry_delay_seconds: 60, credentials_available: false }))
      .toEqual({ allowInternalAddresses: false, maximumAttempts: 8, retryDelaySeconds: 60, credentialsAvailable: false })
    expect(normalizeConnectionTest({
      success: false, address: '203.0.113.5:587', encrypted: true, authenticated: false, extensions: ['STARTTLS'],
      duration_milliseconds: 231, error: { kind: 'authentication', message: '5.7.8 bad credentials', code: 535 },
    })).toEqual({
      success: false, address: '203.0.113.5:587', encrypted: true, authenticated: false, extensions: ['STARTTLS'],
      durationMilliseconds: 231, error: { kind: 'authentication', message: '5.7.8 bad credentials', code: 535 },
    })
    expect(normalizeConnectionTest({
      success: false, address: null, encrypted: false, authenticated: false, extensions: [], duration_milliseconds: 3,
      error: { kind: 'mystery', message: 'odd' },
    }).error).toEqual({ kind: 'permanent', message: 'odd', code: null })
  })

  /**
   * Routing actions default to enabled and relay actions never send forward recipients.
   */
  it('normalizes and encodes routing actions', () => {
    expect(normalizeRoutingAction({
      id: 'action-1', scope_id: 'scope-1', smtp_outbound_id: 'outbound-1', action_type: 'forward',
      forward_recipients: ['qa@example.com'], enabled: true, created_at: 2, updated_at: 3,
    })).toEqual({
      id: 'action-1', scopeId: 'scope-1', smtpOutboundId: 'outbound-1', actionType: 'forward',
      forwardRecipients: ['qa@example.com'], enabled: true, createdAt: 2000, updatedAt: 3000,
    })
    expect(routingActionWire({ smtpOutboundId: 'outbound-1', actionType: 'relay', forwardRecipients: ['qa@example.com'] }))
      .toEqual({ smtp_outbound_id: 'outbound-1', action_type: 'relay', forward_recipients: [] })
    expect(routingActionWire({ smtpOutboundId: 'outbound-1', actionType: 'forward', forwardRecipients: [' qa@example.com ', ''], enabled: false }))
      .toEqual({ smtp_outbound_id: 'outbound-1', action_type: 'forward', forward_recipients: ['qa@example.com'], enabled: false })
  })

  /**
   * Deliveries keep their keyset sequence, convert every time and treat unknown states as failures.
   */
  it('normalizes deliveries with recipients and attempts', () => {
    const wire: WireOutboundDeliveryDetail = {
      id: 'delivery-1', seq: 42, message_id: 'message-1', subject: 'Welcome', scope_id: 'scope-1', routing_action_id: 'action-1',
      smtp_outbound_id: 'outbound-1', smtp_outbound_name: 'Pippo', action_type: 'relay', status: 'retrying', envelope_from: null,
      attempt_count: 1, last_error: { kind: 'temporary', message: '451 try later', code: 451 }, created_at: 10, last_attempt_at: 11,
      next_attempt_at: 71, delivered_at: null,
      recipients: [
        { address: 'qa@example.com', status: 'delivered', error: null, response: '250 queued', delivered_at: 11 },
        { address: 'dev@example.com', status: 'bounced', error: null, response: null, delivered_at: null },
      ],
      attempts: [{ number: 1, started_at: 10, finished_at: 11, outcome: 'partial', error: null, delivered_recipients: 1, deferred_recipients: 1, failed_recipients: 0 }],
    }
    const detail = normalizeOutboundDeliveryDetail(wire)
    expect(detail).toMatchObject({
      sequence: 42, status: 'retrying', createdAt: 10_000, lastAttemptAt: 11_000, nextAttemptAt: 71_000, deliveredAt: null,
      lastError: { kind: 'temporary', code: 451 },
    })
    expect(detail.recipients.map((recipient) => recipient.status)).toEqual(['delivered', 'failed'])
    expect(detail.recipients[0]!.deliveredAt).toBe(11_000)
    expect(detail.attempts).toEqual([{ number: 1, startedAt: 10_000, finishedAt: 11_000, outcome: 'partial', error: null, deliveredRecipients: 1, deferredRecipients: 1, failedRecipients: 0 }])
  })

  /**
   * List filters leave out unset values for both IPC objects and HTTP query strings.
   */
  it('encodes delivery list filters', () => {
    expect(outboundDeliveryListWire()).toEqual({})
    expect(outboundDeliveryListWire({ status: 'failed', before: 42, limit: 25 })).toEqual({ status: 'failed', before: 42, limit: 25 })
    expect(outboundDeliveryListQuery({ status: 'failed', before: 42 })).toBe('?status=failed&before=42')
    expect(outboundDeliveryListQuery({})).toBe('')
  })
})
