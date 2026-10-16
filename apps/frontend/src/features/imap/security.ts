import type { ImapAccount, ImapSecurity, ImapServer } from '@/api/contracts'
import { i18n } from '@/i18n'

/**
 * The default IMAP listener address used when no runtime value is configured.
 */
export const DEFAULT_IMAP_LISTEN = '127.0.0.1:1143'

/**
 * Name the connection-security setting mail clients show for this mode.
 */
export function imapSecurityLabel(security: ImapSecurity): string {
  switch (security) {
    case 'implicit': return i18n.global.t('imap.security.sslTls')
    case 'startTlsOptional':
    case 'startTlsRequired': return i18n.global.t('imap.security.startTls')
    case 'none': return i18n.global.t('imap.security.none')
  }
}

/**
 * Explain in one sentence how the connection protects the password in transit.
 */
export function imapSecurityDescription(security: ImapSecurity): string {
  switch (security) {
    case 'implicit': return i18n.global.t('imap.security.description.implicit')
    case 'startTlsRequired': return i18n.global.t('imap.security.description.required')
    case 'startTlsOptional': return i18n.global.t('imap.security.description.optional')
    case 'none': return i18n.global.t('imap.security.description.none')
  }
}

/**
 * Report whether the service offers TLS, which needs a certificate the client must trust.
 */
export function usesTls(security: ImapSecurity): boolean {
  return security !== 'none'
}

/**
 * One copyable connection field shown to the user.
 */
export interface ImapConnectionField {
  key: 'host' | 'port' | 'security' | 'username' | 'password'
  label: string
  value: string
}

/**
 * List the values a mail client asks for. The password appears only when it was just issued;
 * otherwise it is masked because the server never returns it again.
 */
export function imapConnectionFields(server: ImapServer, account: Pick<ImapAccount, 'username'>, password: string | null): ImapConnectionField[] {
  return [
    { key: 'host', label: i18n.global.t('imap.connection.server'), value: server.enabled ? server.host ?? '—' : i18n.global.t('imap.connection.disabled') },
    { key: 'port', label: i18n.global.t('imap.connection.port'), value: server.enabled && server.port !== null ? String(server.port) : '—' },
    { key: 'security', label: i18n.global.t('imap.connection.security'), value: imapSecurityLabel(server.security) },
    { key: 'username', label: i18n.global.t('imap.connection.username'), value: account.username },
    { key: 'password', label: i18n.global.t('imap.connection.password'), value: password ?? '********' },
  ]
}

/**
 * Count the mailboxes an account exposes, for the account list.
 */
export function mailboxCountLabel(account: Pick<ImapAccount, 'mailboxes'>): string {
  const count = account.mailboxes.length
  return i18n.global.t(count === 0 ? 'imap.mailboxCount.zero' : count === 1 ? 'imap.mailboxCount.one' : 'imap.mailboxCount.many', { count })
}
