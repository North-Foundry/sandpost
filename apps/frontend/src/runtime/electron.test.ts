import { describe, expect, it, vi } from 'vitest'
import type { SandpostDesktopBridge, WireDesktopEvent } from '@/api/desktop'
import { createElectronClient } from './electron'

const wireImapFolder = {
  identifier: 'folder-id', account_identifier: 'account-id', name: 'Signups', filter: 'subject contains "welcome"',
  kind: 'private' as const, definition_identifier: null, include_in_inbox: true, created_at: 1_700_000_000, updated_at: 1_700_000_100,
}
const wireImapAccount = {
  identifier: 'account-id', owner_identifier: 'owner', name: 'Apple Mail', username: 'apple-mail', enabled: true, mode: 'independent' as const, mirror_views: false,
  created_at: 1_700_000_000, updated_at: 1_700_000_100, last_used_at: null,
  linked_views: [{ view_identifier: 'view-id', alias: 'Billing' }],
  folders: [wireImapFolder],
  mailboxes: [
    { name: 'INBOX', source: 'inbox' as const, view_identifier: null, folder_identifier: null },
    { name: 'Billing', source: 'view' as const, view_identifier: 'view-id', folder_identifier: null },
  ],
}

/**
 * Build a preload bridge double with representative responses for every adapter operation.
 */
function makeBridge(): SandpostDesktopBridge {
  return {
    runtime: 'electron',
    instanceStatus: vi.fn(async () => ({ initialized: true, setupKeyRequired: false as const })),
    setup: vi.fn(async () => { }),
    login: vi.fn(async () => ({ id: 'owner', name: 'Owner', email: 'owner@example.test', role: 'owner' as const })),
    session: vi.fn(async () => ({ id: 'owner', name: 'Owner', email: 'owner@example.test', role: 'owner' as const })),
    logout: vi.fn(async () => { }),
    inboxes: {
      list: vi.fn(async () => [{ id: 'inbox-id', name: 'Inbox', address: null, system: true }]),
      create: vi.fn(async () => ({ id: 'created-id', name: 'QA', address: 'qa+tag@example.test', system: false })),
    },
    views: {
      list: vi.fn(async () => [{ identifier: 'view-id', owner_identifier: null, name: 'Shared', filter: 'size > 1' }]),
      save: vi.fn(async (input) => input),
      delete: vi.fn(async () => { }),
    },
    folders: {
      list: vi.fn(async () => [{ identifier: 'web-folder', owner_identifier: 'owner', name: 'Review', exposed_accounts: ['account-id'], created_at: 1_700_000_000, updated_at: 1_700_000_100 }]),
      create: vi.fn(async (input) => ({ identifier: 'web-folder', owner_identifier: 'owner', ...input, created_at: 1_700_000_000, updated_at: 1_700_000_100 })),
      update: vi.fn(async (id, input) => ({ identifier: id, owner_identifier: 'owner', ...input, created_at: 1_700_000_000, updated_at: 1_700_000_100 })),
      delete: vi.fn(async () => { }),
      listMessages: vi.fn(async () => []),
    },
    messages: {
      list: vi.fn(async () => [{
        id: 'mail-id', seq: 19, subject: 'Hello', from: [{ address: 'sender@example.test', domain: 'example.test' }],
        to: [], received_at: 42, size: 100, attachment_count: 0, seen: true,
      }]),
      get: vi.fn(async () => ({
        id: 'mail-id', facts: {
          subject: 'Hello', from: [{ address: 'sender@example.test', domain: 'example.test' }], to: [], cc: [],
          received_at: 42, text: 'body', html: '', headers: {},
        },
        attachments: [{ filename: null, content_type: 'application/octet-stream', size: 3, content_hash: 'hash' }],
      })),
      search: vi.fn(async () => []),
      setSeen: vi.fn(async () => ({ changed: 1 })),
      copy: vi.fn(async () => ({ copied: 2 })),
      setFlags: vi.fn(async () => ({ changed: 1 })),
      getRaw: vi.fn(async () => new Uint8Array([0, 255, 42])),
      getAttachment: vi.fn(async () => new Uint8Array([10, 0, 255])),
      delete: vi.fn(async () => { }),
      move: vi.fn(async () => ({ moved: 2 })),
      trash: vi.fn(async () => ({ trashed: 2 })),
    },
    trash: {
      listMessages: vi.fn(async () => [{
        id: 'trashed-id', seq: 7, subject: 'Old', from: [], to: [], received_at: 41, size: 10, attachment_count: 0, seen: true,
      }]),
      empty: vi.fn(async () => ({ deleted: 3 })),
    },
    clipboard: { writeText: vi.fn(async () => { }) },
    events: { subscribe: vi.fn(async () => () => { }) },
    imap: {
      server: vi.fn(async () => ({
        enabled: true, host: 'mail.example.test', port: 993, security: 'implicit' as const, listen_address: '0.0.0.0:993',
      })),
      list: vi.fn(async () => [wireImapAccount]),
      get: vi.fn(async () => wireImapAccount),
      create: vi.fn(async () => ({
        account: wireImapAccount, password: 'imap-password-shown-once',
        server: { enabled: false, host: null, port: null, security: 'none' as const, listen_address: null },
      })),
      update: vi.fn(async (_id: string, input) => ({ ...wireImapAccount, enabled: input.enabled ?? true, mode: input.mode ?? wireImapAccount.mode })),
      delete: vi.fn(async () => { }),
      regenerate: vi.fn(async () => { throw { message: 'IMAP account not found' } }),
      restore: vi.fn(async () => ({ restored: 3 })),
      folders: {
        create: vi.fn(async (_account: string, input) => ({ ...wireImapFolder, ...input })),
        update: vi.fn(async (_account: string, _folder: string, input) => ({ ...wireImapFolder, ...input })),
        delete: vi.fn(async () => { }),
      },
    },
    admin: {
      smtp: {
        server: vi.fn(async () => ({
          listen_address: '127.0.0.1:2526',
          listener: {
            configured_address: '127.0.0.1:2526', bound_address: null, source: 'configuration' as const, state: 'failed' as const,
            error: '127.0.0.1:2526 is already in use by another process', authentication_required: false,
            encryption: 'start_tls_required' as const,
          },
        })),
        list: vi.fn(async () => [{
          identifier: 'access-id', name: 'Staging', username: 'staging-1a2b3c',
          enabled: true, created_at: 1_700_000_000, last_used_at: null,
        }]),
        create: vi.fn(async (input) => ({
          access: {
            identifier: 'access-id', name: input.name, username: input.username ?? 'generated',
            enabled: true, created_at: 1_700_000_000, last_used_at: 1_700_000_060,
          },
          password: 'generated-password-shown-once',
          server: { listen_address: '0.0.0.0:2525', listener: null },
        })),
        regenerate: vi.fn(async () => { throw 'access not found' }),
        update: vi.fn(async (_id: string, input: { enabled: boolean }) => ({
          identifier: 'access-id', name: 'Staging', username: 'staging-1a2b3c',
          enabled: input.enabled, created_at: 1_700_000_000, last_used_at: null,
        })),
        delete: vi.fn(async () => { }),
      },
      scopes: {
        list: vi.fn(async () => []),
        save: vi.fn(async (input) => ({ ...input, policy_version: 3 })),
        delete: vi.fn(async () => { }),
        preview: vi.fn(async () => []),
        assignments: vi.fn(async () => ['user-id']),
        assign: vi.fn(async () => { }),
        unassign: vi.fn(async () => { }),
      },
      users: {
        list: vi.fn(async () => [{ identifier: 'user-id', name: 'Reader', email: 'reader@example.test', role: 'member' as const, mail_access: 'scoped' as const, personal_filter: null }]),
        create: vi.fn(async (input) => ({ identifier: 'new-id', personal_filter: null, ...input, role: 'member' as const, mail_access: 'all' as const })),
        update: vi.fn(async () => ({ identifier: 'user-id', name: 'Renamed', email: 'reader@example.test', role: 'admin' as const, mail_access: 'all' as const, personal_filter: null })),
        delete: vi.fn(async () => { }),
      },
      validateQuery: vi.fn(async () => ({ valid: false, error: { message: 'expected value', position: 11 } })),
    },
  }
}

describe('Electron bridge adapter', () => {
  it('forwards lifecycle calls through preload without exposing a token', async () => {
    const bridge = makeBridge()
    const client = createElectronClient(bridge)
    const setup = { bootstrapSecret: 'secret', name: 'Owner', email: 'owner@example.test', password: 'password' }
    expect(await client.instanceStatus()).toEqual({ initialized: true, setupKeyRequired: false })
    await expect(client.authorizeSetup({ setupKey: 'unused' })).rejects.toThrow('Setup key authorization is not required')
    await client.setup(setup)
    expect(await client.login({ email: setup.email, password: setup.password })).toMatchObject({ role: 'owner' })
    expect(await client.session()).toMatchObject({ email: setup.email })
    await client.logout()
    expect(bridge.setup).toHaveBeenCalledWith(setup)
    expect(bridge.login).toHaveBeenCalledWith({ email: setup.email, password: setup.password })
    expect(bridge.logout).toHaveBeenCalledOnce()
  })

  it('turns non-Error preload failures into readable errors', async () => {
    const bridge = makeBridge()
    vi.mocked(bridge.login).mockRejectedValueOnce({ message: 'Account is locked.' })
    await expect(createElectronClient(bridge).login({ email: 'a@b.test', password: 'secret' }))
      .rejects.toThrow('Account is locked.')
  })

  it('passes keyset options and normalizes the unchanged Rust DTO shapes', async () => {
    const bridge = makeBridge()
    const client = createElectronClient(bridge)
    expect(await client.messages.list({ before: 20, limit: 10 })).toMatchObject([{
      id: 'mail-id', sequence: 19, from: ['sender@example.test'], receivedAt: 42000,
    }])
    expect(bridge.messages.list).toHaveBeenCalledWith({ before: 20, limit: 10 })
    expect(await client.messages.get('mail-id')).toMatchObject({
      id: 'mail-id', subject: 'Hello', text: 'body', receivedAt: 42000,
      attachments: [{ filename: null, contentType: 'application/octet-stream', size: 3 }],
    })
    expect(bridge.messages.get).toHaveBeenCalledWith('mail-id')
    expect(await client.messages.getRaw('mail-id')).toEqual(new Uint8Array([0, 255, 42]))
    expect(bridge.messages.getRaw).toHaveBeenCalledWith('mail-id')
  })

  it('normalizes saved views both ways and forwards view and query list options', async () => {
    const bridge = makeBridge()
    const client = createElectronClient(bridge)
    expect(await client.views.list()).toEqual([{ id: 'view-id', ownerId: null, name: 'Shared', filter: 'size > 1', icon: null }])
    const view = { id: 'view-id', ownerId: 'user-id', name: 'Mine', filter: 'subject contains "x"', icon: 'bug' }
    expect(await client.views.save(view)).toEqual(view)
    expect(bridge.views.save).toHaveBeenCalledWith({
      identifier: 'view-id', owner_identifier: 'user-id', name: 'Mine', filter: 'subject contains "x"', icon: 'bug',
    })
    await client.views.delete('view-id')
    expect(bridge.views.delete).toHaveBeenCalledWith('view-id')
    await client.messages.list({ viewId: 'view-id', query: 'size > 1', limit: 5 })
    expect(bridge.messages.list).toHaveBeenLastCalledWith({ viewId: 'view-id', query: 'size > 1', limit: 5 })
    await client.messages.list({ query: 'true', limit: 5 })
    expect(bridge.messages.list).toHaveBeenLastCalledWith({ query: 'true', limit: 5 })
  })

  it('normalizes Web folders and forwards folder lists, scoped pages, and placement batches', async () => {
    const bridge = makeBridge()
    const client = createElectronClient(bridge)
    expect(await client.folders.list()).toMatchObject([{ id: 'web-folder', ownerId: 'owner', exposedAccounts: ['account-id'], createdAt: 1_700_000_000_000 }])
    expect(await client.messages.list({ folderId: 'web-folder', before: 9, limit: 4 })).toEqual([])
    expect(bridge.folders.listMessages).toHaveBeenCalledWith('web-folder', { before: 9, limit: 4, query: undefined })
    expect(await client.messages.move(['message-a', 'message-b'], null)).toBe(2)
    expect(bridge.messages.move).toHaveBeenCalledWith({ identifiers: ['message-a', 'message-b'], folderIdentifier: null })
  })

  it.each([true, false])('forwards Seen %s through the dedicated bridge and returns changed count', async (seen) => {
    const bridge = makeBridge();
    const client = createElectronClient(bridge);
    expect(await client.messages.setSeen(['mail-id'], seen)).toBe(1);
    expect(bridge.messages.setSeen).toHaveBeenCalledWith({ identifiers: ['mail-id'], seen });
    vi.mocked(bridge.messages.setSeen).mockRejectedValueOnce({ message: 'Forbidden' });
    await expect(client.messages.setSeen(['mail-id'], seen)).rejects.toThrow('Forbidden');
  });

  it('forwards primary COPY and flag operations through dedicated bridge calls', async () => {
    const bridge = makeBridge()
    const client = createElectronClient(bridge)
    expect(await client.messages.copy(['mail-id'], null)).toBe(2)
    expect(bridge.messages.copy).toHaveBeenCalledWith({ identifiers: ['mail-id'], folderIdentifier: null })
    const flags = { seen: false, answered: true, flagged: false, draft: false, deleted: false, keywords: ['replied'] }
    expect(await client.messages.setFlags({ identifiers: ['mail-id'], operation: 'replace', flags })).toBe(1)
    expect(bridge.messages.setFlags).toHaveBeenCalledWith({ identifiers: ['mail-id'], operation: 'replace', flags })
  })

  it('forwards Trash batches, Trash pages, and emptying to their dedicated bridge calls', async () => {
    const bridge = makeBridge()
    const client = createElectronClient(bridge)
    expect(await client.messages.trash(['message-a', 'message-b'])).toBe(2)
    expect(bridge.messages.trash).toHaveBeenCalledWith({ identifiers: ['message-a', 'message-b'] })
    expect(await client.trash.listMessages({ before: 9, limit: 4, query: 'size > 1' })).toMatchObject([{ id: 'trashed-id', sequence: 7 }])
    expect(bridge.trash.listMessages).toHaveBeenCalledWith({ before: 9, limit: 4, query: 'size > 1' })
    expect(await client.trash.empty()).toBe(3)
    vi.mocked(bridge.trash.empty).mockRejectedValueOnce({ message: 'Viewers cannot delete mail' })
    await expect(client.trash.empty()).rejects.toThrow('Viewers cannot delete mail')
  })

  it('maps desktop wire events and returns the bridge teardown function', async () => {
    const bridge = makeBridge()
    const listener = vi.fn()
    const teardown = vi.fn()
    vi.mocked(bridge.events.subscribe).mockImplementation(async (receive) => {
      for (const event of [
        { type: 'ready' }, { type: 'message', id: 'mail-id', seq: 19 },
        { type: 'resync' }, { type: 'disconnected' },
      ] satisfies WireDesktopEvent[]) receive(event)
      return teardown
    })
    const stop = await createElectronClient(bridge).events.subscribe(listener)
    expect(listener.mock.calls.map(([event]) => event)).toEqual([
      { type: 'ready' }, { type: 'message', id: 'mail-id', sequence: 19 },
      { type: 'resync' }, { type: 'disconnected' },
    ])
    stop()
    expect(teardown).toHaveBeenCalledOnce()
  })

  it('forwards inbox, search, delete, and attachment operations', async () => {
    const bridge = makeBridge()
    const client = createElectronClient(bridge)
    expect(await client.inboxes.list()).toMatchObject([{ address: null }])
    expect(await client.inboxes.create({ name: 'QA', address: 'qa+tag@example.test' })).toMatchObject({ id: 'created-id', address: 'qa+tag@example.test' })
    expect(bridge.inboxes.create).toHaveBeenCalledWith('QA', 'qa+tag@example.test')
    expect(await client.messages.search('term', { before: 8, limit: 4 })).toEqual([])
    expect(bridge.messages.search).toHaveBeenCalledWith('term', 8, 4)
    await client.messages.delete('mail-id')
    expect(bridge.messages.delete).toHaveBeenCalledWith('mail-id')
    expect(await client.messages.getAttachment('mail-id', 1)).toEqual(new Uint8Array([10, 0, 255]))
    expect(bridge.messages.getAttachment).toHaveBeenCalledWith('mail-id', 1)
  })

  it('propagates bridge operation and subscription failures unchanged', async () => {
    const bridge = makeBridge()
    const failure = new Error('IPC unavailable')
    vi.mocked(bridge.messages.list).mockRejectedValueOnce(failure)
    vi.mocked(bridge.messages.get).mockRejectedValueOnce(failure)
    vi.mocked(bridge.messages.getRaw).mockRejectedValueOnce(failure)
    vi.mocked(bridge.messages.search).mockRejectedValueOnce(failure)
    vi.mocked(bridge.messages.delete).mockRejectedValueOnce(failure)
    vi.mocked(bridge.messages.getAttachment).mockRejectedValueOnce(failure)
    vi.mocked(bridge.events.subscribe).mockRejectedValueOnce(failure)
    const client = createElectronClient(bridge)
    await expect(client.messages.list()).rejects.toBe(failure)
    await expect(client.messages.get('id')).rejects.toBe(failure)
    await expect(client.messages.getRaw('id')).rejects.toBe(failure)
    await expect(client.messages.search('x')).rejects.toBe(failure)
    await expect(client.messages.delete('id')).rejects.toBe(failure)
    await expect(client.messages.getAttachment('id', 0)).rejects.toBe(failure)
    await expect(client.events.subscribe(() => { })).rejects.toBe(failure)
  })

  it('normalizes administration DTOs in both directions', async () => {
    const bridge = makeBridge()
    const client = createElectronClient(bridge)
    const scope = { id: 'scope-id', parentId: null, name: 'EU', description: null, filter: 'to.domain == "eu.test"', position: 2 }
    expect(await client.admin.scopes.save(scope)).toEqual({ ...scope, policyVersion: 3 })
    expect(bridge.admin.scopes.save).toHaveBeenCalledWith({
      id: 'scope-id', parent: null, name: 'EU', description: null,
      filter: 'to.domain == "eu.test"', position: 2, policy_version: 0,
    })
    await client.admin.scopes.preview({ parentId: 'parent-id', filter: '' })
    expect(bridge.admin.scopes.preview).toHaveBeenCalledWith({ parent: 'parent-id', filter: '', limit: 20 })
    expect(await client.admin.scopes.assignments('scope-id')).toEqual(['user-id'])
    expect(await client.admin.users.list()).toEqual([{ id: 'user-id', name: 'Reader', email: 'reader@example.test', role: 'member', mailAccess: 'scoped' }])
    await client.admin.users.update('user-id', { role: 'viewer', mailAccess: 'all' })
    expect(bridge.admin.users.update).toHaveBeenCalledWith('user-id', { role: 'viewer', mail_access: 'all' })
    expect(await client.admin.validateQuery('subject == ')).toEqual({ valid: false, error: { message: 'expected value', position: 11 } })
  })

  it('adapts SMTP access administration and passes issued passwords through untouched', async () => {
    const bridge = makeBridge()
    const client = createElectronClient(bridge)
    expect(await client.admin.smtp.server()).toEqual({
      listenAddress: '127.0.0.1:2526',
      listener: {
        configuredAddress: '127.0.0.1:2526', boundAddress: null, source: 'configuration', state: 'failed',
        error: '127.0.0.1:2526 is already in use by another process', authenticationRequired: false,
        encryption: 'startTlsRequired',
      },
    })
    expect(await client.admin.smtp.list()).toEqual([{
      id: 'access-id', name: 'Staging', username: 'staging-1a2b3c',
      enabled: true, createdAt: 1_700_000_000_000, lastUsedAt: null, requiresEncryption: false, allowedMechanisms: ['plain', 'login'],
    }])
    const issued = await client.admin.smtp.create({ name: 'CI', username: '  ' })
    expect(bridge.admin.smtp.create).toHaveBeenCalledWith({ name: 'CI', username: null })
    expect(issued.password).toBe('generated-password-shown-once')
    expect(issued.access.lastUsedAt).toBe(1_700_000_060_000)
    expect(issued.server).toEqual({ listenAddress: '0.0.0.0:2525', listener: null })
    expect((await client.admin.smtp.setEnabled('access-id', false)).enabled).toBe(false)
    expect(bridge.admin.smtp.update).toHaveBeenCalledWith('access-id', { enabled: false })
    await client.admin.smtp.updateRules('access-id', { requiresEncryption: true, allowedMechanisms: ['login'] })
    expect(bridge.admin.smtp.update).toHaveBeenLastCalledWith('access-id', { requires_encryption: true, allowed_mechanisms: ['login'] })
    await expect(client.admin.smtp.regeneratePassword('missing')).rejects.toThrow('access not found')
    await client.admin.smtp.delete('access-id')
    expect(bridge.admin.smtp.delete).toHaveBeenCalledWith('access-id')
  })

  it('adapts IMAP accounts and folders and passes issued passwords through untouched', async () => {
    const bridge = makeBridge()
    const client = createElectronClient(bridge)
    expect(await client.imap.server()).toEqual({
      enabled: true, host: 'mail.example.test', port: 993, security: 'implicit', listenAddress: '0.0.0.0:993',
    })
    const [account] = await client.imap.list()
    expect(account).toMatchObject({
      id: 'account-id', ownerId: 'owner', mode: 'independent', createdAt: 1_700_000_000_000, updatedAt: 1_700_000_100_000, lastUsedAt: null,
      linkedViews: [{ viewId: 'view-id', alias: 'Billing' }],
      mailboxes: [
        { name: 'INBOX', source: 'inbox', viewId: null, folderId: null },
        { name: 'Billing', source: 'view', viewId: 'view-id', folderId: null },
      ],
    })
    expect(account!.folders[0]).toEqual({
      id: 'folder-id', accountId: 'account-id', kind: 'private', definitionId: null, name: 'Signups', filter: 'subject contains "welcome"', includeInInbox: true,
      createdAt: 1_700_000_000_000, updatedAt: 1_700_000_100_000,
    })
    const issued = await client.imap.create({ name: 'Apple Mail', username: ' ', mode: 'independent' })
    expect(bridge.imap.create).toHaveBeenCalledWith({
      name: 'Apple Mail', username: null, mode: 'independent',
    })
    expect(issued.password).toBe('imap-password-shown-once')
    expect(issued.server).toEqual({ enabled: false, host: null, port: null, security: 'none', listenAddress: null })
    expect((await client.imap.update('account-id', { enabled: false })).enabled).toBe(false)
    expect(bridge.imap.update).toHaveBeenCalledWith('account-id', { enabled: false })
    await client.imap.update('account-id', { mode: 'mirror' })
    expect(bridge.imap.update).toHaveBeenLastCalledWith('account-id', { mode: 'mirror' })
    expect(await client.imap.restoreHidden('account-id')).toBe(3)
    await expect(client.imap.regeneratePassword('missing')).rejects.toThrow('IMAP account not found')
    await client.imap.folders.create('account-id', { name: 'Signups', filter: '' })
    expect(bridge.imap.folders.create).toHaveBeenCalledWith('account-id', { name: 'Signups', filter: '', include_in_inbox: true })
    expect((await client.imap.folders.update('account-id', 'folder-id', { name: 'Welcome', filter: '', includeInInbox: false })).includeInInbox).toBe(false)
    expect(bridge.imap.folders.update).toHaveBeenCalledWith('account-id', 'folder-id', { name: 'Welcome', filter: '', include_in_inbox: false })
    await client.imap.folders.delete('account-id', 'folder-id')
    expect(bridge.imap.folders.delete).toHaveBeenCalledWith('account-id', 'folder-id')
    await client.imap.delete('account-id')
    expect(bridge.imap.delete).toHaveBeenCalledWith('account-id')
  })
})
