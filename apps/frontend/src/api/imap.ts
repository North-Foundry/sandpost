import type {
  ImapAccount,
  ImapAccountInput,
  ImapAccountUpdate,
  ImapFolderInput,
  ImapLinkedView,
  ImapLocalFolder,
  ImapMailbox,
  ImapSecurity,
  ImapServer,
  IssuedImapCredentials,
} from './contracts'
import type {
  WireImapAccount,
  WireImapAccountInput,
  WireImapAccountUpdate,
  WireImapFolderInput,
  WireImapLinkedView,
  WireImapLocalFolder,
  WireImapMailbox,
  WireImapSecurity,
  WireImapServer,
  WireIssuedImapCredentials,
} from './wire'

/**
 * Mailbox sources the server reports; anything else is shown as a local folder.
 */
const MAILBOX_SOURCES = new Set<ImapMailbox['source']>([
  'inbox',
  'trash',
  'view',
  'folder',
])

/**
 * Convert the Rust security mode; unknown or missing values mean no TLS.
 */
function normalizeImapSecurity(
  security: WireImapSecurity | undefined
): ImapSecurity {
  switch (security) {
    case 'start_tls_optional':
      return 'startTlsOptional'
    case 'start_tls_required':
      return 'startTlsRequired'
    case 'implicit':
      return 'implicit'
    default:
      return 'none'
  }
}

/**
 * Convert the IMAP service's connection details.
 */
export function normalizeImapServer(
  representation: WireImapServer
): ImapServer {
  return {
    enabled: Boolean(representation.enabled),
    host: representation.host == null ? null : String(representation.host),
    port: representation.port == null ? null : Number(representation.port),
    security: normalizeImapSecurity(representation.security),
    listenAddress:
      representation.listen_address == null
        ? null
        : String(representation.listen_address),
  }
}

/**
 * Convert a linked Smart Folder, treating a blank alias as following the folder's name.
 */
function normalizeLinkedView(
  representation: WireImapLinkedView
): ImapLinkedView {
  return {
    viewId: String(representation.view_identifier),
    alias:
      representation.alias == null || !String(representation.alias).trim()
        ? null
        : String(representation.alias),
  }
}

/**
 * Convert an IMAP-local folder, turning Unix seconds into milliseconds.
 */
export function normalizeImapFolder(
  representation: WireImapLocalFolder
): ImapLocalFolder {
  return {
    id: String(representation.identifier),
    accountId: String(representation.account_identifier),
    kind: representation.kind ?? 'private',
    definitionId:
      representation.definition_identifier == null
        ? null
        : String(representation.definition_identifier),
    name: String(representation.name),
    filter: String(representation.filter ?? ''),
    includeInInbox: Boolean(representation.include_in_inbox),
    createdAt: Number(representation.created_at) * 1000,
    updatedAt: Number(representation.updated_at) * 1000,
  }
}

/**
 * Convert one exposed mailbox name and its source.
 */
function normalizeMailbox(representation: WireImapMailbox): ImapMailbox {
  return {
    name: String(representation.name),
    source: MAILBOX_SOURCES.has(representation.source)
      ? representation.source
      : 'folder',
    viewId:
      representation.view_identifier == null
        ? null
        : String(representation.view_identifier),
    folderId:
      representation.folder_identifier == null
        ? null
        : String(representation.folder_identifier),
  }
}

/**
 * Convert a Rust IMAP account, turning Unix seconds into milliseconds and keeping mailbox order.
 */
export function normalizeImapAccount(
  representation: WireImapAccount
): ImapAccount {
  return {
    id: String(representation.identifier),
    ownerId: String(representation.owner_identifier),
    name: String(representation.name),
    username: String(representation.username),
    enabled: Boolean(representation.enabled),
    mode:
      representation.mode ??
      (representation.mirror_views === false ? 'independent' : 'mirror'),
    createdAt: Number(representation.created_at) * 1000,
    updatedAt: Number(representation.updated_at) * 1000,
    lastUsedAt:
      representation.last_used_at == null
        ? null
        : Number(representation.last_used_at) * 1000,
    linkedViews: (representation.linked_views ?? []).map(normalizeLinkedView),
    folders: (representation.folders ?? []).map(normalizeImapFolder),
    mailboxes: (representation.mailboxes ?? []).map(normalizeMailbox),
  }
}

/**
 * Convert issued IMAP credentials; the password is passed through untouched and never stored.
 */
export function normalizeIssuedImapCredentials(
  representation: WireIssuedImapCredentials
): IssuedImapCredentials {
  return {
    account: normalizeImapAccount(representation.account),
    password: String(representation.password),
    server: normalizeImapServer(representation.server),
  }
}

/**
 * Encode a new IMAP account; blank optional fields let the server choose.
 */
export function imapAccountWire(input: ImapAccountInput): WireImapAccountInput {
  return {
    name: input.name,
    username: input.username?.trim() || null,
    mode: input.mode ?? 'mirror',
  }
}

/**
 * Encode a partial account change, sending only the fields that change.
 */
export function imapAccountUpdateWire(
  changes: ImapAccountUpdate
): WireImapAccountUpdate {
  return {
    ...(changes.name !== undefined ? { name: changes.name } : {}),
    ...(changes.enabled !== undefined ? { enabled: changes.enabled } : {}),
    ...(changes.mode !== undefined ? { mode: changes.mode } : {}),
  }
}

/**
 * Encode an IMAP-local folder; folders join INBOX unless asked otherwise.
 */
export function imapFolderWire(input: ImapFolderInput): WireImapFolderInput {
  return {
    name: input.name,
    filter: input.filter,
    include_in_inbox: input.includeInInbox ?? true,
  }
}
