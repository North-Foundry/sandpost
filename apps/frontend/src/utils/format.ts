import { i18n } from '@/i18n'
import { computed } from 'vue'

/**
 * Reuse formatters across mail rows, rebuilding them only when the language changes.
 */
const formats = computed(() => {
  const locale = i18n.global.locale.value
  return {
    time: new Intl.DateTimeFormat(locale, { hour: 'numeric', minute: '2-digit' }),
    day: new Intl.DateTimeFormat(locale, { month: 'short', day: 'numeric' }),
    year: new Intl.DateTimeFormat(locale, { month: 'short', day: 'numeric', year: '2-digit' }),
    stamp: new Intl.DateTimeFormat(locale, { month: 'short', day: 'numeric', hour: '2-digit', minute: '2-digit' }),
    full: new Intl.DateTimeFormat(locale, { dateStyle: 'medium', timeStyle: 'short' }),
    integer: new Intl.NumberFormat(locale, { maximumFractionDigits: 0 }),
    decimal: new Intl.NumberFormat(locale, { minimumFractionDigits: 1, maximumFractionDigits: 1 }),
  }
})

/**
 * Formats a list timestamp as a time today, a day this year, or a short dated label.
 */
export function formatListDate(timestamp: number, now = new Date()): string {
  const date = new Date(timestamp)

  if (date.toDateString() === now.toDateString()) return formats.value.time.format(date)
  if (date.getFullYear() === now.getFullYear()) return formats.value.day.format(date)
  return formats.value.year.format(date)
}

/**
 * Formats a compact day and time for search results.
 */
export function formatStamp(timestamp: number): string {
  return formats.value.stamp.format(new Date(timestamp))
}

/**
 * Formats a complete medium date and short time for message details.
 */
export function formatFullDate(timestamp: number): string {
  return formats.value.full.format(new Date(timestamp))
}

/**
 * Format attachment sizes using powers of 1024 and the displayed B, KB and MB
 * labels, rounding kilobytes to an integer and displaying megabytes with one decimal.
 */
export function formatBytes(bytes: number): string {
  if (bytes < 1024) return `${formats.value.integer.format(bytes)} B`
  if (bytes < 1024 * 1024) return `${formats.value.integer.format(bytes / 1024)} KB`
  return `${formats.value.decimal.format(bytes / (1024 * 1024))} MB`
}

/**
 * Returns a display name from a mailbox, preferring its quoted name over the local part.
 */
export function mailboxName(mailbox: string): string {
  const bracket = mailbox.indexOf('<')

  if (bracket >= 0) return mailbox.slice(0, bracket).trim().replace(/^"|"$/g, '') || mailbox
  return mailbox.split('@')[0]!.replace(/[._-]/g, ' ')
}

/**
 * Returns a compact sender label for a list of mailboxes.
 */
export function sendersLabel(senders: string[]): string {
  return senders.map(mailboxName).join(', ') || i18n.global.t('common.unknownSender')
}

/**
 * Returns the uppercase initial used for a sender avatar.
 */
export function senderInitial(senders: string[]): string {
  return (mailboxName(senders[0] ?? '').trim().charAt(0) || '?').toUpperCase()
}
