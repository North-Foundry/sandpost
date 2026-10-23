import { describe, expect, it } from 'vitest'
import type { SessionUser, SmtpOutboundInput } from '@/api/contracts'
import { createMockAdmin } from './admin'

const owner: SessionUser = { id: 'owner-id', name: 'Owner', email: 'owner@example.test', role: 'owner' }
const member: SessionUser = { ...owner, id: 'member-id', role: 'member' }

const input: SmtpOutboundInput = {
  name: 'Pippo', host: 'smtp.example.com', port: 587, security: 'starttls', authenticationType: 'plain', username: 'user',
  password: 'secret', overrideFrom: null, envelopeFrom: null, connectTimeoutSeconds: null, smtpTimeoutSeconds: null,
  tlsVerify: true, heloName: null, maxConnections: null, allowedRecipients: ['*@example.com'],
}

/**
 * Create a mock administration client with one saved scope.
 */
async function adminWithScope(user: () => SessionUser | null = () => owner) {
  const admin = createMockAdmin(user, [])
  if (user()?.role === 'owner') {
    await admin.scopes.save({ id: 'scope-1', parentId: null, name: 'Signups', description: null, filter: '', position: 0 })
  }
  return admin
}

describe('mock SMTP Outbound administration', () => {
  /**
   * Store only whether a password exists, default to disabled and keep the stored password on update.
   */
  it('creates outbounds without exposing passwords', async () => {
    const admin = await adminWithScope()
    const created = await admin.outbound.create(input)
    expect(created).toMatchObject({ name: 'Pippo', enabled: false, passwordConfigured: true, routingActionCount: 0 })
    expect(JSON.stringify(await admin.outbound.list())).not.toContain('secret')
    const updated = await admin.outbound.update(created.id, { ...input, password: null, port: 2525, enabled: true })
    expect(updated).toMatchObject({ port: 2525, enabled: true, passwordConfigured: true })
    expect((await admin.outbound.update(created.id, { ...input, authenticationType: 'none', password: null })).passwordConfigured).toBe(false)
  })

  /**
   * Names are unique without regard to case.
   */
  it('rejects duplicate names', async () => {
    const admin = await adminWithScope()
    await admin.outbound.create(input)
    await expect(admin.outbound.create({ ...input, name: 'PIPPO' })).rejects.toMatchObject({ status: 409 })
  })

  /**
   * An outbound used by routing actions cannot be deleted until the actions are gone.
   */
  it('refuses to delete an outbound in use', async () => {
    const admin = await adminWithScope()
    const outbound = await admin.outbound.create(input)
    const action = await admin.routing.create('scope-1', { smtpOutboundId: outbound.id, actionType: 'relay', forwardRecipients: [] })
    expect((await admin.outbound.get(outbound.id)).routingActionCount).toBe(1)
    await expect(admin.outbound.delete(outbound.id)).rejects.toMatchObject({ status: 409 })
    await admin.routing.delete(action.id)
    await admin.outbound.delete(outbound.id)
    expect(await admin.outbound.list()).toEqual([])
  })

  /**
   * Forward recipients must be allowed by the outbound's allowlist.
   */
  it('validates forward recipients against the allowlist', async () => {
    const admin = await adminWithScope()
    const outbound = await admin.outbound.create(input)
    await expect(admin.routing.create('scope-1', { smtpOutboundId: outbound.id, actionType: 'forward', forwardRecipients: ['boss@other.test'] }))
      .rejects.toThrow('not allowed')
    await expect(admin.routing.create('scope-1', { smtpOutboundId: outbound.id, actionType: 'forward', forwardRecipients: [] }))
      .rejects.toThrow('recipients')
    const action = await admin.routing.create('scope-1', { smtpOutboundId: outbound.id, actionType: 'forward', forwardRecipients: ['qa@example.com'] })
    expect(action).toMatchObject({ scopeId: 'scope-1', enabled: true, forwardRecipients: ['qa@example.com'] })
    expect(await admin.routing.list('scope-1')).toHaveLength(1)
  })

  /**
   * Connection tests are deterministic and reuse a stored password for drafts of an existing outbound.
   */
  it('returns deterministic connection tests', async () => {
    const admin = await adminWithScope()
    const outbound = await admin.outbound.create(input)
    expect(await admin.outbound.test(outbound.id)).toMatchObject({ success: true, encrypted: true, authenticated: true, error: null })
    expect((await admin.outbound.testDraft({ ...input, password: null })).error?.kind).toBe('authentication')
    expect((await admin.outbound.testDraft({ ...input, password: null }, outbound.id)).success).toBe(true)
    expect((await admin.outbound.testDraft({ ...input, host: 'smtp.invalid' })).error?.kind).toBe('connection')
  })

  /**
   * Actions of a deleted scope disappear with it and no longer block deleting the outbound.
   */
  it('forgets actions of deleted scopes', async () => {
    const admin = await adminWithScope()
    const outbound = await admin.outbound.create(input)
    await admin.routing.create('scope-1', { smtpOutboundId: outbound.id, actionType: 'relay', forwardRecipients: [] })
    await admin.scopes.delete('scope-1')
    await admin.outbound.delete(outbound.id)
  })

  /**
   * Members cannot use outbound administration, and the mock never has deliveries.
   */
  it('is limited to Owners and has no deliveries', async () => {
    const admin = await adminWithScope(() => member)
    await expect(admin.outbound.list()).rejects.toThrow('forbidden')
    const ownerAdmin = await adminWithScope()
    expect(await ownerAdmin.deliveries.forMessage('message-1')).toEqual([])
    expect(await ownerAdmin.deliveries.list({ status: 'failed' })).toEqual([])
  })
})
