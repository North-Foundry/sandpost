import { createMemoryHistory } from 'vue-router'
import { describe, expect, it, vi } from 'vitest'
import type { SandpostClient } from '@/api/contracts'
import { createAppRouter } from './index'

describe('authentication route guards', () => {
  it('routes through setup, login, then mail as instance and session state change', async () => {
    let initialized = false
    let authenticated = false
    const client = {
      instanceStatus: vi.fn(async () => ({ initialized })),
      session: vi.fn(async () => authenticated ? {
        id: 'owner', name: 'Owner', email: 'owner@example.test', role: 'owner' as const,
      } : null),
    } as unknown as SandpostClient
    const router = createAppRouter(client, createMemoryHistory())

    await router.push('/mail')
    expect(router.currentRoute.value.name).toBe('setup')
    initialized = true
    await router.push('/login')
    expect(router.currentRoute.value.name).toBe('login')
    await router.push('/mail')
    expect(router.currentRoute.value.name).toBe('login')
    authenticated = true
    await router.push('/setup')
    expect(router.currentRoute.value.name).toBe('mail')
  })

  it('does not repeat status checks when selecting messages inside the mail interface', async () => {
    const client = {
      instanceStatus: vi.fn(async () => ({ initialized: true })),
      session: vi.fn(async () => ({ id: 'owner', name: 'Owner', email: 'owner@example.test', role: 'owner' as const })),
    } as unknown as SandpostClient
    const router = createAppRouter(client, createMemoryHistory())

    await router.push('/mail')
    await router.push('/mail/first')
    await router.push('/mail/second')
    await router.push('/mail')
    expect(router.currentRoute.value.name).toBe('mail')
    expect(client.instanceStatus).toHaveBeenCalledOnce()
    expect(client.session).toHaveBeenCalledOnce()
  })

  it('lets the user reach login when the instance status endpoint is unavailable', async () => {
    const client = { instanceStatus: vi.fn(async () => { throw new Error('offline') }) } as unknown as SandpostClient
    const router = createAppRouter(client, createMemoryHistory())
    await router.push('/mail')
    expect(router.currentRoute.value.name).toBe('login')
  })

  it('lets any signed-in role reach IMAP accounts and requires a session for them', async () => {
    let authenticated = false
    const client = {
      instanceStatus: vi.fn(async () => ({ initialized: true })),
      session: vi.fn(async () => authenticated ? {
        id: 'viewer', name: 'Viewer', email: 'viewer@example.test', role: 'viewer' as const,
      } : null),
    } as unknown as SandpostClient
    const router = createAppRouter(client, createMemoryHistory())

    await router.push('/imap')
    expect(router.currentRoute.value.name).toBe('login')
    authenticated = true
    await router.push('/imap')
    expect(router.currentRoute.value.name).toBe('imap')
    const checks = vi.mocked(client.session).mock.calls.length
    await router.push('/imap/account-id')
    expect(router.currentRoute.value).toMatchObject({ name: 'imap-account', params: { id: 'account-id' } })
    /*
     * Moving between IMAP pages keeps the shell mounted without repeating the session check.
     */
    expect(client.session).toHaveBeenCalledTimes(checks)
  })

  /**
   * Require a session for the SMTP Outbound page and keep the shell mounted between admin pages.
   */
  it('guards the SMTP Outbound administration route like the other admin pages', async () => {
    let authenticated = false
    const client = {
      instanceStatus: vi.fn(async () => ({ initialized: true })),
      session: vi.fn(async () => authenticated ? {
        id: 'owner', name: 'Owner', email: 'owner@example.test', role: 'owner' as const,
      } : null),
    } as unknown as SandpostClient
    const router = createAppRouter(client, createMemoryHistory())

    await router.push('/admin/smtp-outbound')
    expect(router.currentRoute.value.name).toBe('login')
    authenticated = true
    await router.push('/admin/smtp-outbound')
    expect(router.currentRoute.value.name).toBe('admin-smtp-outbound')
    const checks = vi.mocked(client.session).mock.calls.length
    await router.push('/admin/smtp-access')
    await router.push('/admin/smtp-outbound')
    expect(router.currentRoute.value.name).toBe('admin-smtp-outbound')
    expect(client.session).toHaveBeenCalledTimes(checks)
  })
})
