import { describe, expect, it, vi } from 'vitest'
import type { ImapAccount, SandpostClient } from '@/api/contracts'
import { i18n } from '@/i18n'
import { describeRestoration, useImapAccountActions } from './accountActions'

const account = {
  id: 'account-id', ownerId: 'owner', name: 'Apple Mail', username: 'apple-mail', enabled: true, mode: 'mirror',
  createdAt: 0, updatedAt: 0, lastUsedAt: null, linkedViews: [], folders: [], mailboxes: [],
} satisfies ImapAccount

describe('IMAP account actions', () => {
  it('describes restorations with and without restored messages', () => {
    expect(describeRestoration(0)).toBe('No hidden messages to restore.')
    expect(describeRestoration(1, 'Mail')).toBe('1 hidden message restored in Mail. They reappear the next time the mail client synchronizes.')
    expect(describeRestoration(3)).toMatch(/^3 hidden messages restored\./)
  })

  it('updates restoration messages when the active locale changes', () => {
    const originalLocale = i18n.global.locale.value
    try {
      i18n.global.locale.value = 'fr'
      expect(describeRestoration(1, 'Mail')).toBe('1 message masqué restauré dans Mail. Il réapparaîtra à la prochaine synchronisation du client de messagerie.')
      expect(describeRestoration(0)).toBe('Aucun message masqué à restaurer.')
    } finally {
      i18n.global.locale.value = originalLocale
    }
  })

  it('shows regenerated credentials once, forgets them on close, and reports failures inline', async () => {
    const changed = vi.fn()
    const deleted = vi.fn()
    const client = {
      imap: {
        regeneratePassword: vi.fn(async () => ({ account, password: 'shown-once', server: { enabled: true, host: 'h', port: 1, security: 'none', listenAddress: null } })),
        restoreHidden: vi.fn(async () => 2),
        delete: vi.fn(async () => { throw Object.assign(new Error('item not found'), { detail: 'item not found' }) }),
      },
    } as unknown as SandpostClient
    const actions = useImapAccountActions(client, { changed, deleted })

    actions.confirmRegenerate(account)
    await actions.regenerate()
    expect(actions.regenerateTarget.value).toBeNull()
    expect(actions.regenerated.value).toBe(true)
    expect(actions.issued.value?.password).toBe('shown-once')
    expect(changed).toHaveBeenCalledWith(account)
    actions.credentialsClosed()
    expect(actions.issued.value).toBeNull()

    actions.confirmRestore(account)
    await actions.restore()
    expect(actions.restoreResult.value).toEqual({ accountName: 'Apple Mail', restored: 2 })

    actions.confirmDelete(account)
    await actions.remove()
    expect(actions.deleteTarget.value).toEqual(account)
    expect(actions.deleteError.value).toBe('Item not found')
    expect(deleted).not.toHaveBeenCalled()
  })
})
