import type {
  ImapAccount, ImapAccountMode, ImapLocalFolder, ImapMailbox, ImapServer, IssuedImapCredentials, SandpostImapClient, SavedView, SessionUser, WebFolder,
} from '@/api/contracts'
import { mockPassword, validateMockQuery } from './admin'

/**
 * The application's per-user and per-account bounds.
 */
const MAXIMUM_ACCOUNTS_PER_USER = 25
const MAXIMUM_FOLDERS_PER_ACCOUNT = 100
const USERNAME = /^[A-Za-z0-9][A-Za-z0-9._+@-]{2,63}$/

/**
 * The runtime's default loopback IMAP listener, without TLS.
 */
const MOCK_SERVER: ImapServer = { enabled: true, host: '127.0.0.1', port: 1143, security: 'none', listenAddress: '127.0.0.1:1143' }

/**
 * Mock account state; like the server, only a fake hash is kept, never the password.
 */
interface StoredAccount {
  id: string
  ownerId: string
  name: string
  username: string
  enabled: boolean
  mode: ImapAccountMode
  createdAt: number
  updatedAt: number
  lastUsedAt: number | null
  folders: ImapLocalFolder[]
  passwordHash: string
}

/**
 * Raise an error with the same `detail` property the HTTP client attaches to API failures.
 */
function fail(detail: string): never {
  throw Object.assign(new Error(detail), { detail })
}

/**
 * Validate a display name like the application: 1 to 100 printable characters.
 */
function validatedName(value: string): string {
  const name = value.trim()
  if (!name || Array.from(name).length > 100 || /[\u0000-\u001f\u007f]/.test(name)) fail('name must contain 1 to 100 printable characters')
  return name
}

/**
 * Turn a display name into a flat mailbox name, as the server's catalog does.
 */
function sanitized(name: string): string {
  const flat = Array.from(name, (character) => character === '/' || /[\u0000-\u001f\u007f]/.test(character) ? '-' : character).join('').trim()
  return flat || 'Untitled'
}

/**
 * Reserve a unique mailbox name, appending a counter on a case-insensitive collision.
 */
function uniqueName(taken: string[], wanted: string): string {
  let candidate = wanted
  for (let counter = 2; taken.some((existing) => existing.toLowerCase() === candidate.toLowerCase()); counter += 1) {
    candidate = `${wanted} (${counter})`
  }
  taken.push(candidate)
  return candidate
}

/**
 * Order mailbox candidates by case-insensitive name, then identifier, like the server.
 */
function byName<T extends { name: string; id: string }>(first: T, second: T): number {
  const firstName = first.name.toLowerCase()
  const secondName = second.name.toLowerCase()
  return firstName < secondName ? -1 : firstName > secondName ? 1 : first.id.localeCompare(second.id)
}

/**
 * Plan the mailboxes an account exposes: INBOX, Trash, Smart Folders, then local folders.
 */
function planMailboxes(account: Pick<StoredAccount, 'mode' | 'folders'>, visibleViews: SavedView[]): ImapMailbox[] {
  const taken = ['INBOX', 'Trash']
  const mailboxes: ImapMailbox[] = [
    { name: 'INBOX', source: 'inbox', viewId: null, folderId: null },
    { name: 'Trash', source: 'trash', viewId: null, folderId: null },
  ]
  const views = account.mode === 'mirror' ? visibleViews.map((view) => ({ id: view.id, name: sanitized(view.name) })) : []
  for (const view of views.sort(byName)) {
    mailboxes.push({ name: uniqueName(taken, view.name), source: 'view', viewId: view.id, folderId: null })
  }
  for (const folder of [...account.folders].sort(byName)) {
    mailboxes.push({ name: uniqueName(taken, sanitized(folder.name)), source: 'folder', viewId: null, folderId: folder.id })
  }
  return mailboxes
}

/**
 * In-memory IMAP accounts for UI development without a server, scoped to the signed-in user.
 */
export function createMockImap(activeUser: () => SessionUser | null, visibleViews: () => SavedView[], visibleWebFolders: () => WebFolder[]): SandpostImapClient {
  const accounts: StoredAccount[] = []

  /**
   * Resolve the current user or raise the API-shaped authentication error.
   */
  const requireUser = () => activeUser() ?? fail('authentication required')

  /**
   * Find an account the caller owns; another user's account is not found, whatever their role.
   */
  const owned = (id: string): StoredAccount => {
    const user = requireUser()
    return accounts.find((account) => account.id === id && account.ownerId === user.id) ?? fail('item not found')
  }

  /**
   * Copy current Web folder names for Mirror, while Independent uses its retained creation snapshot.
   */
  const webFolderProjections = (accountId: string, kind: 'manual' | 'private'): ImapLocalFolder[] => visibleWebFolders().map((folder) => ({
    id: kind === 'manual' ? `mirror-${accountId}-${folder.id}` : crypto.randomUUID(), accountId, kind,
    definitionId: kind === 'manual' ? folder.id : null, name: folder.name, filter: 'true', includeInInbox: false,
    createdAt: folder.createdAt, updatedAt: folder.updatedAt,
  }))

  /**
   * Hide credentials and select the organization state for the active mode.
   */
  const details = ({ passwordHash: _hash, ...account }: StoredAccount): ImapAccount => {
    const folders = account.mode === 'mirror' ? webFolderProjections(account.id, 'manual') : account.folders
    return structuredClone({ ...account, folders, linkedViews: [], mailboxes: planMailboxes({ ...account, folders }, visibleViews()) })
  }

  /**
   * Rotate the account secret and return its public projection with the password once.
   */
  const issue = (account: StoredAccount): IssuedImapCredentials => {
    const password = mockPassword()
    account.passwordHash = `mock-hash-${password.length}-${crypto.randomUUID()}`
    return { account: details(account), password, server: { ...MOCK_SERVER } }
  }

  /**
   * Only Independent accounts can manage local Smart Folders.
   */
  const independent = (id: string): StoredAccount => {
    const account = owned(id)
    if (account.mode !== 'independent') fail('Smart Folders can only be managed in Independent mode')
    return account
  }

  /**
   * Validate a local folder's name and query, rejecting a duplicate name in the same account.
   */
  const validatedFolder = (account: StoredAccount, input: { name: string; filter: string }, except?: string) => {
    const name = validatedName(input.name)
    const validation = validateMockQuery(input.filter)
    if (!validation.valid) fail(validation.error!.message)
    if (account.folders.some((folder) => folder.id !== except && folder.name.toLowerCase() === name.toLowerCase())) fail('conflict')
    return { name, filter: input.filter.trim() }
  }

  return {

    /**
     * Return the loopback mock server settings after requiring authentication.
     */
    async server() { requireUser(); return { ...MOCK_SERVER } },

    /**
     * List only the active user's accounts as detached public projections.
     */
    async list() {
      const user = requireUser()
      return accounts.filter((account) => account.ownerId === user.id).map(details)
    },

    /**
     * Return one owned account, hiding its stored fake password hash.
     */
    async get(id) { return details(owned(id)) },

    /**
     * Create a validated account, snapshotting current Web folders for Independent mode.
     */
    async create(input) {
      const user = requireUser()
      const name = validatedName(input.name)
      const requested = input.username?.trim()
      if (requested && !USERNAME.test(requested)) fail('username must contain 3 to 64 letters, digits, or . _ - + @ and start with a letter or digit')
      if (requested && accounts.some((account) => account.username === requested)) fail('conflict')
      if (accounts.filter((account) => account.ownerId === user.id).length >= MAXIMUM_ACCOUNTS_PER_USER) {
        fail(`a user can own at most ${MAXIMUM_ACCOUNTS_PER_USER} IMAP accounts`)
      }
      const slug = name.toLowerCase().replace(/[^a-z0-9]+/g, '-').replace(/^-|-$/g, '').slice(0, 32) || 'imap'
      const now = Date.now()
      const account: StoredAccount = {
        id: crypto.randomUUID(), ownerId: user.id, name, username: requested || `${slug}-${crypto.randomUUID().slice(0, 6)}`,
        enabled: true, mode: input.mode ?? 'mirror', createdAt: now, updatedAt: now, lastUsedAt: null,
        folders: [], passwordHash: '',
      }
      account.folders = webFolderProjections(account.id, 'private')
      accounts.push(account)
      return issue(account)
    },

    /**
     * Update supplied account settings and refresh its modification time.
     */
    async update(id, changes) {
      const account = owned(id)
      const name = changes.name === undefined ? account.name : validatedName(changes.name)
      Object.assign(account, {
        name,
        enabled: changes.enabled ?? account.enabled,
        mode: changes.mode ?? account.mode,
        updatedAt: Date.now(),
      })
      return details(account)
    },

    /**
     * Delete an owned account and its in-memory folder state.
     */
    async delete(id) {
      owned(id)
      accounts.splice(accounts.findIndex((account) => account.id === id), 1)
    },

    /**
     * Generate and return a replacement one-time password for an owned account.
     */
    async regeneratePassword(id) { return issue(owned(id)) },
    /*
     * No mail client connects to the mock, so there are no hidden messages to expunge.
     */

    /**
     * Confirm ownership; this mock has no connected mail client and restores zero messages.
     */
    async restoreHidden(id) { owned(id); return 0 },
    folders: {

      /**
       * Create an Independent-mode Smart Folder after query, name, uniqueness, and count checks.
       */
      async create(accountId, input) {
        const account = independent(accountId)
        const { name, filter } = validatedFolder(account, input)
        if (account.folders.length >= MAXIMUM_FOLDERS_PER_ACCOUNT) fail(`an IMAP account can have at most ${MAXIMUM_FOLDERS_PER_ACCOUNT} folders`)
        const now = Date.now()
        const folder: ImapLocalFolder = {
          id: crypto.randomUUID(), accountId, kind: 'smart', definitionId: null, name, filter, includeInInbox: input.includeInInbox ?? true, createdAt: now, updatedAt: now,
        }
        account.folders.push(folder)
        return { ...folder }
      },

      /**
       * Update an owned Smart Folder, replacing its filter and inbox-inclusion value.
       */
      async update(accountId, folderId, input) {
        const account = independent(accountId)
        const folder = account.folders.find((candidate) => candidate.id === folderId) ?? fail('item not found')
        Object.assign(folder, validatedFolder(account, input, folderId), { includeInInbox: input.includeInInbox ?? true, updatedAt: Date.now() })
        return { ...folder }
      },

      /**
       * Delete an owned Smart Folder or fail when it is absent.
       */
      async delete(accountId, folderId) {
        const account = independent(accountId)
        const index = account.folders.findIndex((folder) => folder.id === folderId)
        if (index < 0) fail('item not found')
        account.folders.splice(index, 1)
      },
    },
  }
}
