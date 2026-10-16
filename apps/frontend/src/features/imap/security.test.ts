import { describe, expect, it } from 'vitest'
import { imapConnectionFields, imapSecurityLabel, mailboxCountLabel, usesTls } from './security'

describe('IMAP connection helpers', () => {
  it('names the mail-client security setting for each mode', () => {
    expect(imapSecurityLabel('implicit')).toBe('SSL/TLS')
    expect(imapSecurityLabel('startTlsOptional')).toBe('STARTTLS')
    expect(imapSecurityLabel('startTlsRequired')).toBe('STARTTLS')
    expect(imapSecurityLabel('none')).toBe('None')
    expect(usesTls('none')).toBe(false)
    expect(usesTls('startTlsOptional')).toBe(true)
  })

  it('masks the password unless it was just issued and reports a disabled service', () => {
    const server = { enabled: true, host: '127.0.0.1', port: 1143, security: 'none' as const, listenAddress: '127.0.0.1:1143' }
    expect(imapConnectionFields(server, { username: 'mail' }, null).map(({ value }) => value))
      .toEqual(['127.0.0.1', '1143', 'None', 'mail', '********'])
    expect(imapConnectionFields(server, { username: 'mail' }, 'secret').at(-1)!.value).toBe('secret')
    expect(imapConnectionFields({ ...server, enabled: false, host: null, port: null }, { username: 'mail' }, null).slice(0, 2).map(({ value }) => value))
      .toEqual(['IMAP disabled', '—'])
  })

  it('pluralizes the mailbox count', () => {
    const mailbox = { name: 'INBOX', source: 'inbox' as const, viewId: null, folderId: null }
    expect(mailboxCountLabel({ mailboxes: [mailbox] })).toBe('1 mailbox')
    expect(mailboxCountLabel({ mailboxes: [mailbox, { ...mailbox, name: 'Trash', source: 'trash' }] })).toBe('2 mailboxes')
  })
})
