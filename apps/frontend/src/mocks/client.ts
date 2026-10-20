import type { Inbox, MessageFlags, SandpostClient, SavedView, SessionUser, SetupInput, WebFolder, WebFolderInput } from '@/api/contracts'
import { mockCompatibilityReport } from './compatibility'
import { createMockLinkChecks } from './linkChecks'
import { mockAttachments, mockDetail, mockSummaries } from './messages'
import { createMockAdmin, matches } from './admin'
import { createMockImap } from './imap'
import { createMockPersonas } from './personas'
import { normalizeInboxAddress } from '@/api/inboxAddress'

const defaultInbox: Inbox = {
  id: '00000000-0000-0000-0000-000000000000', name: 'Default Inbox', address: null, system: true,
}

/**
 * Recipient inboxes are views with exactly this filter shape, as in the Rust application.
 */
const RECIPIENT_FILTER = /^envelope\.to\.address == "([^"]+)"$/
const uuidPattern = /^[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}$/i

/**
 * Encode UTF-8 fixture data as MIME base64 wrapped at 76 characters.
 */
function encodeMime(value: string): string {
  return btoa(String.fromCharCode(...new TextEncoder().encode(value))).match(/.{1,76}/g)?.join('\r\n') ?? ''
}

/**
 * Adapt the fixed synthetic inbox for UI work without a running server.
 */
export function createMockClient(): SandpostClient {
  const views = new Map<string, SavedView>()
  const folders = new Map<string, WebFolder>()
  const placements = new Map<string, string>()
  const copies = new Map<string, Set<string>>()
  const inboxCopies = new Set<string>()

  /**
   * Present recipient views through the compatibility inbox surface, Default Inbox first.
   */
  const listInboxes = (): Inbox[] => [defaultInbox, ...[...views.values()].flatMap((view) => {
    const address = RECIPIENT_FILTER.exec(view.filter)?.[1]
    return address ? [{ id: view.id, name: view.name, address, system: false }] : []
  })]
  const deleted = new Set<string>()
  const messageFlags = new Map<string, MessageFlags>()

  /**
   * Return a detached flag record, using the all-false fixture state when unset.
   */
  const flagsFor = (id: string): MessageFlags => structuredClone(messageFlags.get(id) ?? {
    seen: false, answered: false, flagged: false, draft: false, deleted: false, keywords: [],
  })

  /**
   * Overlay the current seen bit and complete flags on a message summary.
   */
  const withFlags = <T extends { id: string; seen: boolean }>(message: T): T & { flags: MessageFlags } => {
    const flags = flagsFor(message.id)
    return { ...message, seen: flags.seen, flags }
  }

  /**
   * Messages in the user's Trash: hidden from every other list and from search until restored or deleted.
   */
  const trashed = new Set<string>()

  /**
   * Resync listeners receive a ready event on subscription and resync after mock mutations.
   */
  const listeners = new Set<(event: { type: 'resync' }) => void>()
  const mockImap = createMockImap(() => activeUser, () => [...views.values()], () => [...folders.values()])
  let nextInboxId = 1
  let initialized = false
  const setupAuthorizations = new Map<string, number>()
  let owner: { user: SessionUser; password: string } | null = null
  let activeUser: SessionUser | null = null

  /**
   * Notify a snapshot of listeners; one listener failure does not stop later listeners.
   */
  const notify = () => {
    for (const listener of [...listeners]) {
      try { listener({ type: 'resync' }) } catch (error) { console.error('Mock event listener failed:', error) }
    }
  }

  /**
   * Resolve a fixture detail unless the message has been permanently deleted.
   */
  const getMessage = (id: string) => !deleted.has(id) ? mockDetail(id) : undefined
  const linkChecks = createMockLinkChecks(getMessage)

  return {

    /**
     * Report whether mock setup has completed and whether a setup key is needed.
     */
    async instanceStatus() { return { initialized, setupKeyRequired: !initialized } },

    /**
     * Issue a ten-minute setup authorization after checking the fixed mock setup key.
     */
    async authorizeSetup({ setupKey }) {
      if (initialized) throw Object.assign(new Error('This instance is already initialized.'), { code: 'already_initialized' })
      if (setupKey !== 'sandpost-test-secret') throw Object.assign(new Error('Invalid setup key.'), { code: 'authentication_failed' })
      const authorization = `mock-setup-${crypto.randomUUID()}`
      setupAuthorizations.set(authorization, Date.now() + 600_000)
      return { authorization }
    },

    /**
     * Validate and store the single mock owner, then invalidate outstanding setup authorizations.
     */
    async setup(input: SetupInput) {
      if (initialized) throw Object.assign(new Error('This instance is already initialized.'), { code: 'already_initialized' })
      if (input.bootstrapSecret !== 'sandpost-test-secret' && (setupAuthorizations.get(input.bootstrapSecret) ?? 0) <= Date.now()) {
        throw Object.assign(new Error('Invalid bootstrap secret.'), { code: 'authentication_failed' })
      }
      if (!input.name.trim() || Array.from(input.name.trim()).length > 100 || /[\u0000-\u001f\u007f]/.test(input.name)) {
        throw new Error('name must contain 1 to 100 printable characters')
      }
      if (!/^[^\s@]+@[^\s@]+$/.test(input.email.trim()) || new TextEncoder().encode(input.email.trim()).length > 254) {
        throw new Error('enter a valid email address')
      }
      const passwordLength = new TextEncoder().encode(input.password).length
      if (passwordLength < 12 || passwordLength > 1024) throw new Error('password must contain 12 to 1024 bytes')
      const email = input.email.trim().toLowerCase()
      owner = {
        user: { id: 'mock-owner', name: input.name.trim(), email, role: 'owner' },
        password: input.password,
      }
      initialized = true
      setupAuthorizations.clear()
    },

    /**
     * Authenticate the configured owner using a normalized email and exact password match.
     */
    async login({ email, password }) {
      if (!owner || owner.user.email !== email.trim().toLowerCase() || owner.password !== password) {
        throw new Error('Email or password is incorrect.')
      }
      activeUser = { ...owner.user }
      return { ...activeUser }
    },

    /**
     * Return a detached active-user record, or null when signed out.
     */
    async session() { return activeUser ? { ...activeUser } : null },

    /**
     * Clear the active mock session.
     */
    async logout() { activeUser = null },
    inboxes: {

      /**
       * List the default inbox and recipient views projected as compatibility inboxes.
       */
      async list() { return listInboxes().map((inbox) => ({ ...inbox })) },

      /**
       * Validate a complete recipient address and create its saved-view projection.
       */
      async create({ name, address }) {
        const normalizedName = name.trim()
        if (!normalizedName || normalizedName.length > 100) {
          throw new Error('Invalid inbox name or address')
        }
        const normalizedAddress = normalizeInboxAddress(address)
        if (listInboxes().some((inbox) => inbox.address === normalizedAddress)) throw new Error('Inbox address already exists')
        const inbox = { id: `10000000-0000-4000-8000-${String(nextInboxId++).padStart(12, '0')}`, name: normalizedName, address: normalizedAddress, system: false }
        views.set(inbox.id, {
          id: inbox.id, ownerId: activeUser?.id ?? null, name: normalizedName,
          filter: `envelope.to.address == "${normalizedAddress}"`,
        })
        notify()
        return { ...inbox }
      },
    },
    views: {

      /**
       * List saved views for the signed-in user as detached values.
       */
      async list() {
        if (!activeUser) throw new Error('authentication required')
        return [...views.values()].map((view) => ({ ...view }))
      },

      /**
       * Validate and insert or replace a view, notifying subscribers after the mutation.
       */
      async save(view) {
        if (!activeUser) throw new Error('authentication required')
        if (!uuidPattern.test(view.id)) throw new Error('invalid view')
        const name = view.name.trim()
        if (!name || Array.from(name).length > 100) throw new Error('name must contain 1 to 100 printable characters')
        const saved = { ...view, name, icon: view.icon ?? null }
        views.set(saved.id, saved)
        notify()
        return { ...saved }
      },

      /**
       * Delete a saved view and notify subscribers, or fail if it is absent.
       */
      async delete(id) {
        if (!views.delete(id)) throw new Error(`Mock view not found: ${id}`)
        notify()
      },
    },
    messages: {

      /**
       * Apply approximate filters and placement rules, then return a sequence-paged summary copy.
       */
      async list(options = {}) {
        const limit = options.limit ?? 50
        validatePage(options.before, limit)
        if (options.inboxId !== undefined && options.inboxId !== defaultInbox.id && !views.has(options.inboxId)) {
          throw new Error(`Mock inbox not found: ${options.inboxId}`)
        }
        const viewId = options.viewId ?? (options.inboxId !== defaultInbox.id ? options.inboxId : undefined)
        if (viewId !== undefined && !views.has(viewId)) throw new Error(`Mock view not found: ${viewId}`)
        /*
         * The mock matches quoted literals in fixture text; the server evaluates the full DSL.
         */
        const filters = [viewId && views.get(viewId)!.filter, options.query].filter((filter): filter is string => Boolean(filter))
        return mockSummaries
          .filter((message) => !filters.length || matches(message, filters))
          .filter((message) => options.folderId
            ? placements.get(message.id) === options.folderId || copies.get(message.id)?.has(options.folderId)
            : Boolean(options.query) || !viewId && (!placements.has(message.id) || inboxCopies.has(message.id)))
          .filter(({ id }) => !deleted.has(id) && !trashed.has(id))
          .filter((message) => options.before === undefined || message.sequence < options.before)
          .slice(0, limit)
          .map((message) => ({ ...withFlags(message), from: [...message.from], to: [...message.to], envelopeTo: [...(message.envelopeTo ?? message.to)] }))
      },

      /**
       * Search fixture content case-insensitively across summaries, bodies, and header values.
       */
      async search(query, options = {}) {
        const limit = options.limit ?? 50
        validatePage(options.before, limit)
        const normalizedQuery = query.trim()
        const term = asciiLowercase(normalizedQuery)
        if (!normalizedQuery || Array.from(normalizedQuery).length > 256) {
          throw new Error('search query must contain 1 to 256 characters')
        }
        return mockSummaries.filter((summary) => {
          if (deleted.has(summary.id) || trashed.has(summary.id) || (options.before !== undefined && summary.sequence >= options.before)) return false
          const detail = mockDetail(summary.id)!
          return [summary.subject, summary.preview, ...summary.from, ...summary.to, ...detail.cc,
          detail.text, detail.html, ...Object.entries(detail.headers).flatMap(([key, values]) => [key, ...values])]
            .some((value) => value !== undefined && asciiLowercase(value).includes(term))
        }).slice(0, limit).map((message) => ({ ...withFlags(message), envelopeTo: [...(message.envelopeTo ?? message.to)] }))
      },

      /**
       * Set the seen flag on a validated batch and return the count whose value changed.
       */
      async setSeen(identifiers, seen) {
        validateBatch(identifiers)
        if (typeof seen !== 'boolean') throw new Error('Invalid message seen status')
        for (const id of identifiers) if (!getMessage(id)) throw new Error(`Mock message not found: ${id}`)
        let changed = 0
        for (const id of identifiers) {
          const flags = flagsFor(id)
          if (flags.seen !== seen) changed += 1
          flags.seen = seen
          messageFlags.set(id, flags)
        }
        if (changed) notify()
        return changed
      },

      /**
       * Copy messages to a folder or default inbox, rejecting duplicate destinations.
       */
      async copy(identifiers, folderIdentifier) {
        validateBatch(identifiers)
        if (folderIdentifier !== null && !folders.has(folderIdentifier)) throw new Error(`Mock folder not found: ${folderIdentifier}`)
        for (const id of identifiers) if (!getMessage(id) || trashed.has(id)) throw new Error(`Mock message not found: ${id}`)
        if (identifiers.some((id) => folderIdentifier === null
          ? (!placements.has(id) || inboxCopies.has(id))
          : (placements.get(id) === folderIdentifier || copies.get(id)?.has(folderIdentifier)))) {
          throw Object.assign(new Error('A message already exists in the destination folder.'), { code: 'conflict' })
        }
        for (const id of identifiers) {
          if (folderIdentifier === null) inboxCopies.add(id)
          else {
            const destinations = copies.get(id) ?? new Set<string>()
            destinations.add(folderIdentifier)
            copies.set(id, destinations)
          }
        }
        notify()
        return identifiers.length
      },

      /**
       * Add, remove, or replace supported flags and keywords, then return the changed count.
       */
      async setFlags(input) {
        validateBatch(input.identifiers)
        if (!['add', 'remove', 'replace'].includes(input.operation)) throw new Error('Invalid message flags operation')
        if (input.flags.deleted !== false) throw new Error('The Deleted flag cannot be changed from the Web UI')
        for (const id of input.identifiers) if (!getMessage(id) || trashed.has(id)) throw new Error(`Mock message not found: ${id}`)
        let changed = 0
        for (const id of input.identifiers) {
          const current = flagsFor(id)
          const next = input.operation === 'replace'
            ? { ...input.flags, deleted: false, keywords: [...new Set(input.flags.keywords)] }
            : { ...current }
          if (input.operation !== 'replace') {
            for (const key of ['seen', 'answered', 'flagged', 'draft'] as const) {
              if (input.flags[key]) next[key] = input.operation === 'add'
            }
            const keywords = new Set(current.keywords)
            for (const keyword of input.flags.keywords) input.operation === 'add' ? keywords.add(keyword) : keywords.delete(keyword)
            next.keywords = [...keywords]
          }
          if (JSON.stringify(current) !== JSON.stringify(next)) changed += 1
          messageFlags.set(id, next)
        }
        if (changed) notify()
        return changed
      },

      /**
       * Return synthetic message details or fail for an unknown or deleted identifier.
       */
      async get(id) {
        const detail = getMessage(id)
        if (!detail) throw new Error(`Mock message not found: ${id}`)
        return detail
      },

      /**
       * Return the synthetic compatibility report, or HTML absence for plain-text fixtures.
       */
      async htmlCheck(id) {
        const detail = getMessage(id)
        if (!detail) throw new Error(`Mock message not found: ${id}`)
        return mockCompatibilityReport(Boolean(detail.html.trim()))
      },

      /**
       * Return the simulated shared link check state without starting a check.
       */
      async linkCheck(id) {
        if (!activeUser) throw new Error('authentication required')
        return linkChecks.get(id)
      },

      /**
       * Start, or join, a simulated check of one message's links.
       */
      async startLinkCheck(id) {
        if (!activeUser) throw new Error('authentication required')
        return linkChecks.start(id)
      },

      /**
       * Start simulated checks of several messages' links.
       */
      async checkLinks(identifiers) {
        if (!activeUser) throw new Error('authentication required')
        return linkChecks.startMany(identifiers)
      },

      /**
       * Encode a synthetic RFC-style multipart message with base64 text and fixture attachments.
       */
      async getRaw(id) {
        const message = getMessage(id)
        if (!message) throw new Error(`Mock message not found: ${id}`)
        const mixed = `sandpost-mixed-${id}`
        const alternative = `sandpost-alternative-${id}`
        const source = [
          `From: ${message.from.join(', ')}`,
          `To: ${message.to.join(', ')}`,
          ...(message.cc.length ? [`Cc: ${message.cc.join(', ')}`] : []),
          `Subject: ${message.subject}`,
          `Date: ${new Date(message.receivedAt).toUTCString()}`,
          `Message-ID: ${message.headers['message-id']![0]}`,
          'MIME-Version: 1.0',
          `Content-Type: multipart/mixed; boundary="${mixed}"`,
          '',
          `--${mixed}`,
          `Content-Type: multipart/alternative; boundary="${alternative}"`,
          '',
          `--${alternative}`,
          'Content-Type: text/plain; charset=utf-8',
          'Content-Transfer-Encoding: base64',
          '',
          encodeMime(message.text),
          ...(message.html ? [
            `--${alternative}`,
            'Content-Type: text/html; charset=utf-8',
            'Content-Transfer-Encoding: base64',
            '',
            encodeMime(message.html),
          ] : []),
          `--${alternative}--`,
          ...mockAttachments(id).flatMap((attachment) => [
            `--${mixed}`,
            `Content-Type: ${attachment.contentType}; charset=utf-8`,
            `Content-Disposition: attachment; filename="${attachment.filename}"`,
            'Content-Transfer-Encoding: base64',
            '',
            encodeMime(attachment.content),
          ]),
          `--${mixed}--`,
          '',
        ].join('\r\n')
        return new TextEncoder().encode(source)
      },

      /**
       * Permanently delete a valid fixture message and remove it from Trash.
       */
      async delete(id) {
        if (!uuidPattern.test(id) || !getMessage(id)) throw new Error(`Mock message not found: ${id}`)
        deleted.add(id)
        trashed.delete(id)
        notify()
      },

      /**
       * Move a batch into a folder or the default inbox, clearing prior copies and Trash state.
       */
      async move(identifiers, folderIdentifier) {
        validateBatch(identifiers)
        if (folderIdentifier && !folders.has(folderIdentifier)) throw new Error('Mock folder not found')
        for (const id of identifiers) if (!getMessage(id)) throw new Error(`Mock message not found: ${id}`)
        for (const id of identifiers) {
          /*
           * Placing a message anywhere also restores it from Trash, as on the server.
           */
          trashed.delete(id)
          copies.delete(id)
          inboxCopies.delete(id)
          if (folderIdentifier) placements.set(id, folderIdentifier)
          else placements.delete(id)
        }
        notify()
        return identifiers.length
      },

      /**
       * Place a validated batch in Trash without removing its underlying fixture.
       */
      async trash(identifiers) {
        validateBatch(identifiers)
        for (const id of identifiers) if (!getMessage(id)) throw new Error(`Mock message not found: ${id}`)
        for (const id of identifiers) trashed.add(id)
        notify()
        return identifiers.length
      },

      /**
       * Return encoded fixture attachment bytes after validating the message and zero-based index.
       */
      async getAttachment(id, index) {
        if (!Number.isInteger(index) || index < 0) throw new Error('Invalid attachment index')
        if (!getMessage(id)) throw new Error(`Mock message not found: ${id}`)
        const attachment = mockAttachments(id)[index]
        if (!attachment) throw new Error(`Mock attachment not found: ${id}/${index}`)
        return new TextEncoder().encode(attachment.content)
      },
    },
    folders: {

      /**
       * List detached Web folder records and detached exposed-account arrays.
       */
      async list() { return [...folders.values()].map((folder) => ({ ...folder, exposedAccounts: [...folder.exposedAccounts] })) },

      /**
       * Create a Web folder using the mock's shared save and validation path.
       */
      async create(input: WebFolderInput) {
        const folder = await saveMockFolder(null, input)
        return folder
      },

      /**
       * Update an existing Web folder while retaining its identifier and creation time.
       */
      async update(id, input) { return saveMockFolder(id, input) },

      /**
       * Delete a folder, move primary placements to Trash, remove copies, and notify listeners.
       */
      async delete(id) {
        if (!folders.delete(id)) throw new Error(`Mock folder not found: ${id}`)
        for (const [messageId, folderId] of placements) if (folderId === id) {
          placements.delete(messageId)
          trashed.add(messageId)
        }
        for (const destinations of copies.values()) destinations.delete(id)
        notify()
      },

      /**
       * List folder placements and copies with optional approximate filtering and sequence paging.
       */
      async listMessages(id, options = {}) {
        if (!folders.has(id)) throw new Error(`Mock folder not found: ${id}`)
        const limit = options.limit ?? 50
        validatePage(options.before, limit)
        const filters = options.query ? [options.query] : []
        return mockSummaries.filter((message) => (placements.get(message.id) === id || copies.get(message.id)?.has(id)) && !deleted.has(message.id) && !trashed.has(message.id)
          && (options.before === undefined || message.sequence < options.before)
          && (!filters.length || matches(message, filters)))
          .slice(0, limit).map((message) => ({ ...withFlags(message), from: [...message.from], to: [...message.to] }))
      },
    },
    trash: {

      /**
       * List trashed fixture messages with optional approximate filtering and sequence paging.
       */
      async listMessages(options = {}) {
        const limit = options.limit ?? 50
        validatePage(options.before, limit)
        const filters = options.query ? [options.query] : []
        /*
         * Fixture summaries are newest first, preserving the server's keyset order.
         */
        return mockSummaries.filter((message) => trashed.has(message.id) && !deleted.has(message.id)
          && (options.before === undefined || message.sequence < options.before)
          && (!filters.length || matches(message, filters)))
          .slice(0, limit).map((message) => ({ ...withFlags(message), from: [...message.from], to: [...message.to] }))
      },

      /**
       * Permanently delete every trashed message and notify only when the Trash was nonempty.
       */
      async empty() {
        const count = trashed.size
        for (const id of trashed) deleted.add(id)
        trashed.clear()
        if (count) notify()
        return count
      },
    },
    imap: mockImap,
    personas: createMockPersonas(() => activeUser, () => mockSummaries
      .filter(({ id }) => !deleted.has(id) && !trashed.has(id))
      .map((message) => ({ ...withFlags(message), from: [...message.from], to: [...message.to], envelopeTo: [...(message.envelopeTo ?? message.to)] }))),
    admin: createMockAdmin(() => activeUser, mockSummaries),
    events: {

      /**
       * Register a resync callback, deliver ready immediately, and return an unsubscribe function.
       */
      async subscribe(listener) {
        /**
         * Forward mock state-change notifications through the subscribed event listener.
         */
        const resync = (event: { type: 'resync' }) => listener(event)

        listeners.add(resync)
        try { listener({ type: 'ready' }) } catch (error) { console.error('Mock event listener failed:', error) }
        return () => { listeners.delete(resync) }
      },
    },
  }

  /**
   * Validate and save an owned mock folder, preserving its identity and creation time on update.
   */
  async function saveMockFolder(id: string | null, input: WebFolderInput): Promise<WebFolder> {
    const user = activeUser
    if (!user) throw new Error('Authentication required')
    const name = input.name.trim()
    if (!name || name.length > 100) throw new Error('Folder name must contain 1 to 100 characters')
    if ([...folders.values()].some((folder) => folder.id !== id && folder.name.toLowerCase() === name.toLowerCase())) {
      throw Object.assign(new Error('A folder with this name already exists.'), { code: 'conflict' })
    }
    const previous = id ? folders.get(id) : undefined
    if (id && !previous) throw new Error(`Mock folder not found: ${id}`)
    const now = Date.now()
    const saved: WebFolder = {
      id: previous?.id ?? crypto.randomUUID(), ownerId: user.id, name, icon: input.icon ?? null, exposedAccounts: [],
      createdAt: previous?.createdAt ?? now, updatedAt: now,
    }
    folders.set(saved.id, saved)
    notify()
    return { ...saved, exposedAccounts: [...saved.exposedAccounts] }
  }
}

/**
 * Lowercase ASCII letters while leaving every other code point unchanged.
 */
function asciiLowercase(value: string): string {
  return value.replace(/[A-Z]/g, (character) => String.fromCharCode(character.charCodeAt(0) + 32))
}

/**
 * Enforce the server's bound of 1 to 256 distinct messages per placement or Trash request.
 */
function validateBatch(identifiers: string[]): void {
  if (identifiers.length < 1 || identifiers.length > 256) throw new Error('Move 1 to 256 messages at a time')
  if (new Set(identifiers).size !== identifiers.length) throw new Error('Duplicate message identifier')
}

/**
 * Validate the optional exclusive sequence cursor and the supported page-size range.
 */
function validatePage(before: number | undefined, limit: number): void {
  if (before !== undefined && (!Number.isSafeInteger(before) || before < 0)) throw new Error('Invalid before cursor')
  if (!Number.isInteger(limit) || limit < 1 || limit > 100) throw new Error('limit must be between 1 and 100')
}
