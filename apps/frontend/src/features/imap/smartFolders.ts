import type { SavedView } from '@/api/contracts'
import type { MailFolder } from '@/features/shell/workspace'

/**
 * The saved Views an IMAP account can expose: address inboxes and Smart Folders, in sidebar order.
 * Inbox has no View; it is always exposed as INBOX.
 */
export function exposableViews(folders: MailFolder[]): SavedView[] {
  return folders.flatMap((folder) => folder.view ? [folder.view] : [])
}
