import { i18n } from '@/i18n'
import { computed, inject, ref, type ComputedRef, type InjectionKey, type Ref } from 'vue'
import type { Inbox, InboxInput, ImapAccount, Persona, PersonaInput, SandpostClient, SandpostEvent, SavedView, SessionUser, WebFolder, WebFolderInput } from '@/api/contracts'
import { describeError } from '@/utils/errors'
import { collectReadableIds, deleteEach, type BulkDeleteResult } from '@/features/mail/bulkDelete'

/**
 * The virtual all-visible-mail inbox; its nil UUID is the server's Default Inbox identifier.
 */
export const DEFAULT_INBOX: Inbox = { id: '00000000-0000-0000-0000-000000000000', name: 'Default Inbox', address: null, system: true }

/**
 * The caller's Trash is a fixed system mailbox rather than a stored folder, so it has a reserved identifier.
 */
export const TRASH_FOLDER_ID = 'trash'

/**
 * Counting stops once this many unread Inbox messages are found; the sidebar shows larger counts as 999+.
 */
export const UNREAD_COUNT_LIMIT = 1000
const UNREAD_PAGE_SIZE = 100

/**
 * Bursts of mail events and deletions coalesce into one recount after this many milliseconds.
 */
const UNREAD_RECOUNT_DELAY = 500

/**
 * A mail destination in the sidebar.
 *
 * `inbox` is all visible mail, `trash` the messages the user deleted and can still restore, `address` a recipient
 * inbox, and `smart` a saved canonical query. Manual `folder`s are user-owned manual placements, independent from
 * smart filters and address inboxes. A `persona` lists readable mail addressed to one of the user's Personas.
 */
export type MailFolderKind = 'inbox' | 'trash' | 'address' | 'smart' | 'folder' | 'persona'

export interface MailFolder {
  id: string
  kind: MailFolderKind
  name: string

  /**
   * Receiving address of an address inbox.
   */
  address: string | null

  /**
   * Saved view behind an address inbox or smart folder, when the server listed it.
   */
  view: SavedView | null

  /**
   * User-owned explicit-membership folder, when this is a manual folder.
   */
  manual?: WebFolder | null

  /**
   * The Persona behind a Persona view.
   */
  persona?: Persona | null
}

export type ConnectionState = 'connecting' | 'connected' | 'disconnected'

/**
 * Application-wide state shared by the shell, the mail page, and administration pages.
 */
export interface Workspace {
  user: Ref<SessionUser | null>

  /**
   * Owners see the Administration navigation and manage shared smart folders.
   */
  canAdminister: Ref<boolean>

  /**
   * True once the session and administration capability have been resolved.
   */
  capabilitiesLoaded: Ref<boolean>
  connection: Ref<ConnectionState>
  folders: ComputedRef<MailFolder[]>

  /**
   * The user's Personas as mail views, sorted by name; empty when the transport offers no Personas.
   */
  personaFolders: ComputedRef<MailFolder[]>

  /**
   * Whether the transport offers Personas.
   */
  personasAvailable: boolean
  imapAccounts: Ref<ImapAccount[]>
  foldersLoading: Ref<boolean>
  foldersLoaded: Ref<boolean>
  foldersError: Ref<string>

  /**
   * Message identifiers the server currently marks as seen in the loaded Inbox pages.
   */
  viewedMessageIds: Ref<Set<string>>

  /**
   * Unread Inbox messages, exact below the count limit; null until the first count completes.
   */
  inboxUnreadCount: ComputedRef<number | null>

  /**
   * Set shared seen status and refresh the unread count after the server accepts it.
   */
  setSeen(identifiers: string[], seen: boolean): Promise<void>

  /**
   * Recount Inbox unread mail soon, for example after messages were deleted.
   */
  refreshInboxUnread(): void

  /**
   * Permanently delete every message this user can read, reporting progress as `(deleted, total)`. Mail listeners
   * receive a resync afterwards so open lists reload.
   */
  deleteAllMail(onProgress: (deleted: number, total: number) => void): Promise<BulkDeleteResult & { total: number }>

  /**
   * Return the folder with this identifier; unknown identifiers resolve to null.
   */
  findFolder(id: string | null | undefined): MailFolder | null

  /**
   * Reload folder definitions and retain successful results when an individual source fails.
   */
  reloadFolders(): Promise<void>

  /**
   * Create an address inbox and add the returned definition to shared state.
   */
  createAddressInbox(input: InboxInput): Promise<Inbox>

  /**
   * Save a smart folder and update its shared local representation.
   */
  saveView(view: SavedView): Promise<SavedView>

  /**
   * Delete a saved view or address inbox without deleting its mail.
   */
  deleteView(id: string): Promise<void>

  /**
   * Create a user-owned manual folder.
   */
  createManualFolder(input: WebFolderInput): Promise<WebFolder>

  /**
   * Create or update a user-owned manual folder.
   */
  saveManualFolder(id: string | null, input: WebFolderInput): Promise<WebFolder>

  /**
   * Delete a manual folder without deleting its messages.
   */
  deleteManualFolder(id: string): Promise<void>

  /**
   * Create or replace one of the user's Personas.
   */
  savePersona(id: string | null, input: PersonaInput): Promise<Persona>

  /**
   * Enable or disable one of the user's Personas.
   */
  setPersonaEnabled(id: string, enabled: boolean): Promise<Persona>

  /**
   * Delete a Persona and its history without deleting its mail.
   */
  deletePersona(id: string): Promise<void>

  /**
   * Replace one Persona's local copy, for example after reading its newest execution.
   */
  updatePersona(persona: Persona): void

  /**
   * Receive application mail events while mounted; returns the unsubscribe function.
   */
  onMailEvent(listener: (event: SandpostEvent) => void): () => void

  /**
   * End the session and invoke the owning shell's signed-out handler.
   */
  logout(): Promise<void>
}

export const workspaceKey: InjectionKey<Workspace> = Symbol('Workspace')

interface WorkspaceOptions {
  /**
   * Called when the session is missing or after logout.
   */
  onSignedOut(): Promise<unknown> | void
}

/**
 * Build the workspace and the lifecycle functions its owning shell must call on mount and unmount.
 */
export function createWorkspace(client: SandpostClient, options: WorkspaceOptions) {
  const user = ref<SessionUser | null>(null)
  const canAdminister = ref(false)
  const capabilitiesLoaded = ref(false)
  const connection = ref<ConnectionState>('connecting')
  const inboxes = ref<Inbox[]>([DEFAULT_INBOX])
  const views = ref<SavedView[]>([])
  const manualFolders = ref<WebFolder[]>([])
  const personas = ref<Persona[]>([])
  const imapAccounts = ref<ImapAccount[]>([])
  const foldersLoading = ref(false)
  const foldersLoaded = ref(false)
  const foldersError = ref('')
  const viewedMessageIds = ref(new Set<string>())

  /**
   * Newest Inbox message identifiers, newest first, scanned until enough unread ones were found.
   */
  const inboxMessageIds = ref<string[] | null>(null)
  const listeners = new Set<(event: SandpostEvent) => void>()
  let unsubscribeEvents: (() => void) | null = null
  let retryTimer: ReturnType<typeof setTimeout> | null = null
  let stopped = false
  let folderRequest = 0
  let unreadRequest = 0
  let unreadTimer: ReturnType<typeof setTimeout> | null = null

  const folders = computed<MailFolder[]>(() => {
    const byId = new Map(views.value.map((view) => [view.id, view]))
    const addressIds = new Set(inboxes.value.map((inbox) => inbox.id))
    const collator = new Intl.Collator(i18n.global.locale.value, { sensitivity: 'base', numeric: true })
    const addresses = inboxes.value
      .filter((inbox) => !inbox.system)
      .map((inbox): MailFolder => ({ id: inbox.id, kind: 'address', name: byId.get(inbox.id)?.name ?? inbox.name, address: inbox.address, view: byId.get(inbox.id) ?? null, manual: null }))
      .sort((first, second) => collator.compare(first.name, second.name))
    const smart = views.value
      .filter((view) => !addressIds.has(view.id))
      .map((view): MailFolder => ({ id: view.id, kind: 'smart', name: view.name, address: null, view, manual: null }))
      .sort((first, second) => collator.compare(first.name, second.name))
    const manual = manualFolders.value
      .map((folder): MailFolder => ({ id: folder.id, kind: 'folder', name: folder.name, address: null, view: null, manual: folder }))
      .sort((first, second) => collator.compare(first.name, second.name))
    return [
      { id: DEFAULT_INBOX.id, kind: 'inbox', name: i18n.global.t('common.inbox'), address: null, view: null, manual: null },
      { id: TRASH_FOLDER_ID, kind: 'trash', name: i18n.global.t('common.trash'), address: null, view: null, manual: null },
      ...addresses, ...smart, ...manual,
    ]
  })

  const personaFolders = computed<MailFolder[]>(() => {
    const collator = new Intl.Collator(i18n.global.locale.value, { sensitivity: 'base', numeric: true })
    return personas.value
      .map((persona): MailFolder => ({ id: persona.id, kind: 'persona', name: persona.name, address: persona.address, view: null, manual: null, persona }))
      .sort((first, second) => collator.compare(first.name, second.name))
  })

  const inboxUnreadCount = computed(() => {
    const viewed = viewedMessageIds.value
    return inboxMessageIds.value?.filter((id) => !viewed.has(id)).length ?? null
  })

  /**
   * Update shared read status and the unread badge after the server accepts the change.
   */
  async function setSeen(identifiers: string[], seen: boolean): Promise<void> {
    await client.messages.setSeen(identifiers, seen)
    /*
     * A page requested before this write must not overwrite the accepted Seen status.
     */
    unreadRequest += 1
    const updated = new Set(viewedMessageIds.value)
    for (const id of identifiers) seen ? updated.add(id) : updated.delete(id)
    viewedMessageIds.value = updated
    refreshInboxUnread()
  }

  /**
   * Page through Inbox messages, newest first, until the count limit of unread ones is reached or the mail ends.
   * Server Seen flags determine the count. A newer recount supersedes this one; failures keep the last count.
   */
  async function recountInboxUnread(): Promise<void> {
    const request = ++unreadRequest
    const identifiers: string[] = []
    const seenIdentifiers = new Set<string>()
    let unread = 0
    let before: number | undefined
    try {
      while (unread < UNREAD_COUNT_LIMIT) {
        const page = await client.messages.list({ before, limit: UNREAD_PAGE_SIZE })
        if (stopped || request !== unreadRequest) return
        for (const message of page) {
          identifiers.push(message.id)
          if (message.seen) seenIdentifiers.add(message.id)
          else unread += 1
        }
        if (page.length < UNREAD_PAGE_SIZE) break
        before = page.reduce((oldest, message) => Math.min(oldest, message.sequence), Infinity)
      }
      inboxMessageIds.value = identifiers
      viewedMessageIds.value = seenIdentifiers
    } catch { /*
     * The badge keeps its last count until the next event.
     */ }
  }

  /**
   * Permanently delete readable messages, report progress, then refresh unread state and listeners.
   */
  async function deleteAllMail(onProgress: (deleted: number, total: number) => void): Promise<BulkDeleteResult & { total: number }> {
    const ids = await collectReadableIds(client, manualFolders.value.map((folder) => folder.id))
    let deleted = 0
    onProgress(0, ids.length)
    try {
      return { ...await deleteEach(client, ids, () => onProgress(++deleted, ids.length)), total: ids.length }
    } finally {
      if (deleted && !stopped) {
        refreshInboxUnread()
        notify({ type: 'resync' })
      }
    }
  }

  /**
   * Coalesce recount requests that arrive together.
   */
  function refreshInboxUnread(): void {
    if (stopped) return
    if (unreadTimer) clearTimeout(unreadTimer)
    unreadTimer = setTimeout(() => { unreadTimer = null; void recountInboxUnread() }, UNREAD_RECOUNT_DELAY)
  }

  /**
   * Load recipient inboxes and saved views together; a newer request or a creation supersedes older results.
   */
  async function reloadFolders(): Promise<void> {
    const request = ++folderRequest
    foldersLoading.value = true
    const [loadedInboxes, loadedViews, loadedManualFolders, loadedImapAccounts, loadedPersonas] = await Promise.allSettled([
      client.inboxes.list(), client.views.list(), client.folders.list(), client.imap.list(),
      client.personas ? client.personas.list() : Promise.resolve([]),
    ])
    if (stopped || request !== folderRequest) return
    if (loadedPersonas.status === 'fulfilled') personas.value = loadedPersonas.value
    if (loadedInboxes.status === 'fulfilled') {
      inboxes.value = loadedInboxes.value.some((inbox) => inbox.system) ? loadedInboxes.value : [DEFAULT_INBOX, ...loadedInboxes.value]
    }
    if (loadedViews.status === 'fulfilled') views.value = loadedViews.value
    if (loadedManualFolders.status === 'fulfilled') manualFolders.value = loadedManualFolders.value
    if (loadedImapAccounts.status === 'fulfilled') imapAccounts.value = loadedImapAccounts.value
    const failure = [loadedInboxes, loadedViews, loadedManualFolders, loadedImapAccounts, loadedPersonas].find((result) => result.status === 'rejected')
    const reason = failure?.status === 'rejected' ? failure.reason : null
    foldersError.value = reason ? describeError(reason, i18n.global.t('common.foldersLoadFailed')) : ''
    foldersLoading.value = false
    foldersLoaded.value = true
  }

  /**
   * Create a recipient inbox and keep it even if an older list request resolves afterward.
   */
  async function createAddressInbox(input: InboxInput): Promise<Inbox> {
    const created = await client.inboxes.create(input)
    folderRequest += 1
    foldersLoading.value = false
    inboxes.value = [...inboxes.value.filter((inbox) => inbox.id !== created.id), created]
    void refreshViewsQuietly()
    return created
  }

  /**
   * Save a smart folder and replace its local copy.
   */
  async function saveView(view: SavedView): Promise<SavedView> {
    const saved = await client.views.save(view)
    folderRequest += 1
    foldersLoading.value = false
    views.value = [...views.value.filter((candidate) => candidate.id !== saved.id), saved]
    inboxes.value = inboxes.value.map((inbox) => inbox.id === saved.id ? { ...inbox, name: saved.name } : inbox)
    return saved
  }

  /**
   * Delete an address inbox or smart folder; mail itself is never affected.
   */
  async function deleteView(id: string): Promise<void> {
    await client.views.delete(id)
    folderRequest += 1
    foldersLoading.value = false
    views.value = views.value.filter((view) => view.id !== id)
    inboxes.value = inboxes.value.filter((inbox) => inbox.id !== id)
  }

  /**
   * Create or replace a manual folder while retaining it across in-flight list requests.
   */
  async function saveManualFolder(id: string | null, input: WebFolderInput): Promise<WebFolder> {
    const saved = id ? await client.folders.update(id, input) : await client.folders.create(input)
    folderRequest += 1
    foldersLoading.value = false
    manualFolders.value = [...manualFolders.value.filter((folder) => folder.id !== saved.id), saved]
    return saved
  }

  /**
   * Create a new manual folder.
   */
  function createManualFolder(input: WebFolderInput): Promise<WebFolder> {
    return saveManualFolder(null, input)
  }

  /**
   * Delete a manual folder without deleting its messages.
   */
  async function deleteManualFolder(id: string): Promise<void> {
    await client.folders.delete(id)
    folderRequest += 1
    foldersLoading.value = false
    manualFolders.value = manualFolders.value.filter((folder) => folder.id !== id)
  }

  /**
   * Return the transport's Persona client, which the shell only offers when it exists.
   */
  function personaClient() {
    if (!client.personas) throw new Error('Personas are not available in this client')
    return client.personas
  }

  /**
   * Replace a Persona's local copy and keep it across in-flight list requests.
   */
  function rememberPersona(persona: Persona): Persona {
    folderRequest += 1
    foldersLoading.value = false
    personas.value = [...personas.value.filter((candidate) => candidate.id !== persona.id), persona]
    return persona
  }

  /**
   * Create or replace a Persona.
   */
  async function savePersona(id: string | null, input: PersonaInput): Promise<Persona> {
    return rememberPersona(id ? await personaClient().update(id, input) : await personaClient().create(input))
  }

  /**
   * Enable or disable a Persona.
   */
  async function setPersonaEnabled(id: string, enabled: boolean): Promise<Persona> {
    return rememberPersona(await personaClient().setEnabled(id, enabled))
  }

  /**
   * Delete a Persona; its mail stays where it is.
   */
  async function deletePersona(id: string): Promise<void> {
    await personaClient().delete(id)
    folderRequest += 1
    foldersLoading.value = false
    personas.value = personas.value.filter((persona) => persona.id !== id)
  }

  /**
   * Pick up the server's view record for a newly created inbox without disturbing the visible list.
   */
  async function refreshViewsQuietly(): Promise<void> {
    const request = folderRequest
    try {
      const loaded = await client.views.list()
      if (!stopped && request === folderRequest) views.value = loaded
    } catch { /*
     * The inbox stays usable; its view record only enables deletion.
     */ }
  }

  /**
   * Resolve the session, then whether any administration is available to it.
   */
  async function loadCapabilities(): Promise<void> {
    try {
      const session = await client.session()
      if (stopped) return
      if (!session) { await options.onSignedOut(); return }
      user.value = session
      canAdminister.value = session.role === 'owner'
    } catch { /*
     * Administration stays hidden; mail does not depend on it.
     */ } finally {
      capabilitiesLoaded.value = true
    }
  }

  /**
   * Forward an application event to mounted listeners and refresh folders after (re)synchronization.
   */
  function dispatch(event: SandpostEvent): void {
    if (event.type === 'disconnected') { connection.value = 'disconnected' } else {
      connection.value = 'connected'
      if (event.type !== 'message') void reloadFolders()
      refreshInboxUnread()
    }
    notify(event)
  }

  /**
   * Deliver an event to mounted listeners; one failing listener does not stop the others.
   */
  function notify(event: SandpostEvent): void {
    for (const listener of [...listeners]) {
      try { listener(event) } catch (error) { console.error('Mail event listener failed:', error) }
    }
  }

  /**
   * Connect the shared event transport, retrying failed subscriptions with bounded backoff.
   */
  async function connectEvents(attempt = 0): Promise<void> {
    try {
      const unsubscribe = await client.events.subscribe(dispatch)
      if (!stopped) unsubscribeEvents = unsubscribe
      else unsubscribe()
    } catch {
      if (stopped) return
      connection.value = 'disconnected'
      retryTimer = setTimeout(() => { void connectEvents(attempt + 1) }, Math.min(30_000, 1_000 * 2 ** Math.min(attempt, 5)))
    }
  }

  const workspace: Workspace = {
    user,
    canAdminister,
    capabilitiesLoaded,
    connection,
    folders,
    personaFolders,
    personasAvailable: Boolean(client.personas),
    imapAccounts,
    foldersLoading,
    foldersLoaded,
    foldersError,
    viewedMessageIds,
    inboxUnreadCount,
    setSeen,
    refreshInboxUnread,
    deleteAllMail,

    /**
     * Resolve a folder identifier, using the virtual Inbox for an empty identifier.
     */
    findFolder: (id) => {
      const wanted = id || DEFAULT_INBOX.id
      return folders.value.find((folder) => folder.id === wanted) ?? personaFolders.value.find((folder) => folder.id === wanted) ?? null
    },
    reloadFolders,
    createAddressInbox,
    saveView,
    deleteView,
    createManualFolder,
    saveManualFolder,
    deleteManualFolder,
    savePersona,
    setPersonaEnabled,
    deletePersona,

    /**
     * Replace one Persona's local copy without a server request.
     */
    updatePersona(persona) {
      if (personas.value.some((candidate) => candidate.id === persona.id)) rememberPersona(persona)
    },

    /**
     * Subscribe a mounted consumer and return a function that removes its listener.
     */
    onMailEvent(listener) {
      listeners.add(listener)
      return () => { listeners.delete(listener) }
    },

    /**
     * End the client session before notifying the shell that authentication is over.
     */
    async logout() {
      await client.logout()
      await options.onSignedOut()
    },
  }

  return {
    workspace,

    /**
     * Start session, folder, and event loading.
     */
    start(): void {
      void loadCapabilities()
      void reloadFolders()
      void recountInboxUnread()
      void connectEvents()
    },

    /**
     * Stop event delivery and ignore in-flight responses.
     */
    stop(): void {
      stopped = true
      folderRequest += 1
      unreadRequest += 1
      listeners.clear()
      if (retryTimer) clearTimeout(retryTimer)
      if (unreadTimer) clearTimeout(unreadTimer)
      unsubscribeEvents?.()
    },
  }
}

/**
 * Return the shell's workspace.
 */
export function useWorkspace(): Workspace {
  const workspace = inject(workspaceKey)
  if (!workspace) throw new Error('Workspace pages must render inside the application shell')
  return workspace
}
