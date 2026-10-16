import type { ListenerEncryption } from '@/api/contracts'
import { i18n } from '@/i18n'

/**
 * Describe how applications reach the SMTP server, completing "Applications connect to host:port …".
 */
export function connectionProtection(encryption: ListenerEncryption): string {
  switch (encryption) {
    case 'startTlsRequired': return i18n.global.t('admin.encryption.startTlsRequired')
    case 'startTlsOptional': return i18n.global.t('admin.encryption.startTlsOptional')
    case 'implicit': return i18n.global.t('admin.encryption.implicit')
    case 'none': return i18n.global.t('admin.encryption.none')
  }
}

/**
 * Explain whether SMTP AUTH credentials are protected in transit.
 */
export function credentialProtection(encryption: ListenerEncryption): string {
  switch (encryption) {
    case 'startTlsRequired': return i18n.global.t('admin.credentials.startTlsRequired')
    case 'implicit': return i18n.global.t('admin.credentials.implicit')
    case 'startTlsOptional': return i18n.global.t('admin.credentials.startTlsOptional')
    case 'none': return i18n.global.t('admin.credentials.none')
  }
}
