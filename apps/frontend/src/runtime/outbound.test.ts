import { afterEach, describe, expect, it, vi } from 'vitest'
import type { SmtpOutboundInput } from '@/api/contracts'
import type { SandpostDesktopBridge } from '@/api/desktop'
import type { WireOutboundDelivery, WireRoutingAction, WireSmtpOutbound } from '@/api/wire'
import { createElectronClient } from './electron'
import { createHttpClient } from './http'

const wireOutbound: WireSmtpOutbound = {
  id: 'outbound/1', name: 'Pippo', host: 'smtp.example.com', port: 587, security: 'starttls', auth_type: 'plain',
  username: 'user', password_configured: true, enabled: true, override_from: null, envelope_from: null,
  connect_timeout: null, smtp_timeout: null, tls_verify: true, helo_name: null, max_connections: null,
  allowed_recipients: ['*@example.com'], routing_action_count: 0, created_at: 1, updated_at: 2,
}
const wireAction: WireRoutingAction = {
  id: 'action-1', scope_id: 'scope-1', smtp_outbound_id: 'outbound/1', action_type: 'relay', forward_recipients: [],
  enabled: true, created_at: 1, updated_at: 1,
}
const wireDelivery: WireOutboundDelivery = {
  id: 'delivery-1', seq: 7, message_id: 'message-1', subject: 'Hi', scope_id: 'scope-1', routing_action_id: 'action-1',
  smtp_outbound_id: 'outbound/1', smtp_outbound_name: 'Pippo', action_type: 'relay', status: 'failed', envelope_from: null,
  attempt_count: 8, last_error: null, created_at: 1, last_attempt_at: 2, next_attempt_at: null, delivered_at: null, recipients: [],
}
const testResult = { success: true, address: '203.0.113.5:587', encrypted: true, authenticated: true, extensions: [], duration_milliseconds: 10, error: null }
const input: SmtpOutboundInput = {
  name: 'Pippo', host: 'smtp.example.com', port: 587, security: 'starttls', authenticationType: 'plain', username: 'user',
  password: '', overrideFrom: null, envelopeFrom: null, connectTimeoutSeconds: null, smtpTimeoutSeconds: null, tlsVerify: true,
  heloName: null, maxConnections: null, allowedRecipients: [],
}

describe('SMTP Outbound transports', () => {
  afterEach(() => vi.unstubAllGlobals())

  /**
   * Map every operation to its versioned route, without sending a blank password.
   */
  it('sends outbound, routing and delivery requests to the versioned HTTP routes', async () => {
    const responses: unknown[] = [
      { allow_internal_addresses: false, maximum_attempts: 8, retry_delay_seconds: 60, credentials_available: true },
      [wireOutbound], wireOutbound, wireOutbound, wireOutbound, testResult, testResult,
      [wireAction], wireAction, wireAction,
      [wireDelivery], { ...wireDelivery, attempts: [] }, wireDelivery, [wireDelivery],
    ]
    const fetchMock = vi.fn(async () => ({ ok: true, status: 200, json: async () => responses.shift() }))
    vi.stubGlobal('fetch', fetchMock)
    const client = createHttpClient()

    expect((await client.admin.outbound.policy()).maximumAttempts).toBe(8)
    expect((await client.admin.outbound.list())[0]).toMatchObject({ id: 'outbound/1', authenticationType: 'plain', createdAt: 1000 })
    await client.admin.outbound.create(input)
    await client.admin.outbound.update('outbound/1', input)
    await client.admin.outbound.setEnabled('outbound/1', false)
    await client.admin.outbound.test('outbound/1')
    expect((await client.admin.outbound.testDraft(input, 'outbound/1')).durationMilliseconds).toBe(10)
    await client.admin.outbound.delete('outbound/1')
    await client.admin.routing.list('scope-1')
    await client.admin.routing.create('scope-1', { smtpOutboundId: 'outbound/1', actionType: 'relay', forwardRecipients: [] })
    await client.admin.routing.update('action-1', { smtpOutboundId: 'outbound/1', actionType: 'forward', forwardRecipients: ['qa@example.com'], enabled: false })
    await client.admin.routing.delete('action-1')
    expect((await client.admin.deliveries.list({ status: 'failed', before: 7, limit: 25 }))[0]!.sequence).toBe(7)
    expect((await client.admin.deliveries.get('delivery-1')).attempts).toEqual([])
    await client.admin.deliveries.retry('delivery-1')
    await client.admin.deliveries.forMessage('message-1')

    const calls = fetchMock.mock.calls as unknown as [string, RequestInit][]
    expect(calls.map(([path, init]) => [init?.method ?? 'GET', path])).toEqual([
      ['GET', '/api/v1/outbound/policy'],
      ['GET', '/api/v1/smtp/outbounds'],
      ['POST', '/api/v1/smtp/outbounds'],
      ['PUT', '/api/v1/smtp/outbounds/outbound%2F1'],
      ['PUT', '/api/v1/smtp/outbounds/outbound%2F1/enabled'],
      ['POST', '/api/v1/smtp/outbounds/outbound%2F1/test'],
      ['POST', '/api/v1/smtp/outbounds/test'],
      ['DELETE', '/api/v1/smtp/outbounds/outbound%2F1'],
      ['GET', '/api/v1/scopes/scope-1/routing-actions'],
      ['POST', '/api/v1/scopes/scope-1/routing-actions'],
      ['PUT', '/api/v1/routing-actions/action-1'],
      ['DELETE', '/api/v1/routing-actions/action-1'],
      ['GET', '/api/v1/outbound/deliveries?status=failed&before=7&limit=25'],
      ['GET', '/api/v1/outbound/deliveries/delivery-1'],
      ['POST', '/api/v1/outbound/deliveries/delivery-1/retry'],
      ['GET', '/api/v1/messages/message-1/deliveries'],
    ])
    const created = JSON.parse(String(calls[2]![1].body))
    expect(created).not.toHaveProperty('password')
    expect(created).toMatchObject({ auth_type: 'plain', allowed_recipients: [] })
    expect(JSON.parse(String(calls[4]![1].body))).toEqual({ enabled: false })
    expect(JSON.parse(String(calls[6]![1].body))).toMatchObject({ id: 'outbound/1', name: 'Pippo' })
    expect(JSON.parse(String(calls[10]![1].body))).toEqual({
      smtp_outbound_id: 'outbound/1', action_type: 'forward', forward_recipients: ['qa@example.com'], enabled: false,
    })
  })

  /**
   * Forward each call to its preload method, passing the draft identifier separately.
   */
  it('adapts the desktop bridge methods', async () => {
    const outbound = {
      policy: vi.fn(async () => ({ allow_internal_addresses: true, maximum_attempts: 3, retry_delay_seconds: 5, credentials_available: true })),
      list: vi.fn(async () => [wireOutbound]),
      get: vi.fn(async () => wireOutbound),
      create: vi.fn(async () => wireOutbound),
      update: vi.fn(async () => wireOutbound),
      setEnabled: vi.fn(async () => ({ ...wireOutbound, enabled: false })),
      delete: vi.fn(async () => null),
      test: vi.fn(async () => testResult),
      testDraft: vi.fn(async () => testResult),
    }
    const routing = {
      list: vi.fn(async () => [wireAction]),
      create: vi.fn(async () => wireAction),
      update: vi.fn(async () => wireAction),
      delete: vi.fn(async () => null),
    }
    const deliveries = {
      list: vi.fn(async () => [wireDelivery]),
      get: vi.fn(async () => ({ ...wireDelivery, attempts: [] })),
      retry: vi.fn(async () => wireDelivery),
      message: vi.fn(async () => [wireDelivery]),
    }
    const client = createElectronClient({ admin: { outbound, routing, deliveries } } as unknown as SandpostDesktopBridge)

    expect((await client.admin.outbound.policy()).allowInternalAddresses).toBe(true)
    expect((await client.admin.outbound.setEnabled('outbound/1', false)).enabled).toBe(false)
    expect(outbound.setEnabled).toHaveBeenCalledWith('outbound/1', false)
    await client.admin.outbound.testDraft({ ...input, password: 'typed' }, 'outbound/1')
    expect(outbound.testDraft).toHaveBeenCalledWith(expect.objectContaining({ password: 'typed', auth_type: 'plain' }), 'outbound/1')
    expect((outbound.testDraft.mock.calls[0] as unknown[])[0]).not.toHaveProperty('id')
    await client.admin.outbound.testDraft(input)
    expect(outbound.testDraft).toHaveBeenLastCalledWith(expect.not.objectContaining({ password: expect.anything() }), null)
    expect(await client.admin.outbound.delete('outbound/1')).toBeUndefined()
    await client.admin.routing.create('scope-1', { smtpOutboundId: 'outbound/1', actionType: 'relay', forwardRecipients: ['ignored@example.com'] })
    expect(routing.create).toHaveBeenCalledWith('scope-1', { smtp_outbound_id: 'outbound/1', action_type: 'relay', forward_recipients: [] })
    await client.admin.deliveries.list({ status: 'failed' })
    expect(deliveries.list).toHaveBeenCalledWith({ status: 'failed' })
    expect((await client.admin.deliveries.forMessage('message-1'))[0]!.sequence).toBe(7)
    expect(deliveries.message).toHaveBeenCalledWith('message-1')
  })

  /**
   * Older preloads without outbound delivery report a readable error, and messages have no deliveries.
   */
  it('degrades gracefully when the desktop preload predates outbound delivery', async () => {
    const client = createElectronClient({ admin: {} } as unknown as SandpostDesktopBridge)
    await expect(client.admin.outbound.list()).rejects.toThrow('not available')
    await expect(client.admin.routing.list('scope-1')).rejects.toThrow('not available')
    expect(await client.admin.deliveries.forMessage('message-1')).toEqual([])
  })
})
