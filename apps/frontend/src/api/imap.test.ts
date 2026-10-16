import { describe, expect, it } from 'vitest'
import { imapAccountUpdateWire, imapAccountWire, imapFolderWire, normalizeImapAccount, normalizeImapServer } from './imap'

describe('IMAP DTO helpers', () => {
  it('converts account timestamps to milliseconds and keeps the exposed mailbox order', () => {
    expect(normalizeImapAccount({
      identifier: 'a', owner_identifier: 'u', name: 'Mail', username: 'mail', enabled: false, mode: 'independent', mirror_views: true,
      created_at: 2, updated_at: 3, last_used_at: 4,
      linked_views: [{ view_identifier: 'v', alias: '  ' }, { view_identifier: 'w', alias: 'Bills' }],
      folders: [{ identifier: 'f', account_identifier: 'a', kind: 'private', definition_identifier: null, name: 'Local', filter: 'size > 1', include_in_inbox: false, created_at: 5, updated_at: 6 }],
      mailboxes: [
        { name: 'INBOX', source: 'inbox', view_identifier: null, folder_identifier: null },
        { name: 'Trash', source: 'trash', view_identifier: null, folder_identifier: null },
        { name: 'Bills', source: 'view', view_identifier: 'w', folder_identifier: null },
        { name: 'Local', source: 'folder', view_identifier: null, folder_identifier: 'f' },
      ],
    })).toEqual({
      id: 'a', ownerId: 'u', name: 'Mail', username: 'mail', enabled: false, mode: 'independent',
      createdAt: 2000, updatedAt: 3000, lastUsedAt: 4000,
      linkedViews: [{ viewId: 'v', alias: null }, { viewId: 'w', alias: 'Bills' }],
      folders: [{ id: 'f', accountId: 'a', kind: 'private', definitionId: null, name: 'Local', filter: 'size > 1', includeInInbox: false, createdAt: 5000, updatedAt: 6000 }],
      mailboxes: [
        { name: 'INBOX', source: 'inbox', viewId: null, folderId: null },
        { name: 'Trash', source: 'trash', viewId: null, folderId: null },
        { name: 'Bills', source: 'view', viewId: 'w', folderId: null },
        { name: 'Local', source: 'folder', viewId: null, folderId: 'f' },
      ],
    })
  })

  it('reads legacy mirror output only when explicit mode is absent', () => {
    const legacy = {
      identifier: 'a', owner_identifier: 'u', name: 'Mail', username: 'mail', enabled: true,
      created_at: 1, updated_at: 1, last_used_at: null, linked_views: [], folders: [], mailboxes: [],
    };
    expect(normalizeImapAccount({ ...legacy, mirror_views: false }).mode).toBe('independent');
    expect(normalizeImapAccount({ ...legacy, mirror_views: true }).mode).toBe('mirror');
    expect(normalizeImapAccount({ ...legacy, mirror_views: false, mode: 'mirror' }).mode).toBe('mirror');
  });

  it('maps every security mode and a disabled service', () => {
    const server = { enabled: true, host: 'h', port: 1, listen_address: null }
    expect(normalizeImapServer({ ...server, security: 'implicit' }).security).toBe('implicit')
    expect(normalizeImapServer({ ...server, security: 'start_tls_optional' }).security).toBe('startTlsOptional')
    expect(normalizeImapServer({ ...server, security: 'start_tls_required' }).security).toBe('startTlsRequired')
    expect(normalizeImapServer({ ...server, security: 'none' }).security).toBe('none')
    expect(normalizeImapServer({ enabled: false, host: null, port: null, security: 'none', listen_address: null }))
      .toEqual({ enabled: false, host: null, port: null, security: 'none', listenAddress: null })
  })

  it('encodes creation defaults, partial updates, and folder INBOX participation', () => {
    expect(imapAccountWire({ name: 'Mail', username: '  ' })).toEqual({ name: 'Mail', username: null, mode: 'mirror' })
    expect(imapAccountWire({ name: 'Mail', username: ' mail ', mode: 'independent' }))
      .toEqual({ name: 'Mail', username: 'mail', mode: 'independent' })
    expect(imapAccountUpdateWire({ enabled: true })).toEqual({ enabled: true })
    expect(imapAccountUpdateWire({ name: 'New', mode: 'independent' })).toEqual({ name: 'New', mode: 'independent' })
    expect(imapFolderWire({ name: 'Local', filter: '' })).toEqual({ name: 'Local', filter: '', include_in_inbox: true })
    expect(imapFolderWire({ name: 'Local', filter: '', includeInInbox: false })).toEqual({ name: 'Local', filter: '', include_in_inbox: false })
  })
})
