export interface MessageSummary {
  id: string
  sequence: number
  subject: string
  from: string[]
  to: string[]
  receivedAt: number
  size: number
  attachmentCount: number
  seen: boolean

  /**
   * Server-authoritative IMAP-compatible flags; absent only on older server summaries.
   */
  flags?: MessageFlags
  envelopeTo?: string[]
  preview?: string
}

/**
 * Primary Web flags supported by Mirror accounts. `deleted` is reported but cannot be changed here.
 */
export interface MessageFlags {
  seen: boolean
  answered: boolean
  flagged: boolean
  draft: boolean
  deleted: boolean
  keywords: string[]
}

export type MessageFlagOperation = 'add' | 'remove' | 'replace'

export interface Inbox {
  id: string
  name: string
  address: string | null
  system: boolean
}

export interface InboxInput {
  name: string
  address: string
}

/**
 * A saved canonical query, presented as a Smart Folder.
 *
 * Views only organize mail the user may already read; they never grant access.
 * A null owner marks a shared view managed by instance administrators.
 */
export interface SavedView {
  id: string
  ownerId: string | null
  name: string
  filter: string
  icon?: string | null
}

/**
 * A user-owned manual folder; IMAP exposure follows each account's mode.
 */
export interface WebFolder {
  id: string
  ownerId: string
  name: string
  icon?: string | null
  exposedAccounts: string[]
  createdAt: number
  updatedAt: number
}

export interface WebFolderInput {
  name: string
  icon?: string | null
}

export type GlobalRole = 'owner' | 'admin' | 'member' | 'viewer'

/**
 * `all` reads every captured message; `scoped` reads only mail matching assigned scopes.
 */
export type MailAccess = 'all' | 'scoped'

export interface SessionUser {
  id: string
  name: string
  email: string
  role: GlobalRole
}

export interface SetupInput {
  /**
   * Legacy wire field accepting the setup key or its temporary authorization.
   */
  bootstrapSecret: string
  name: string
  email: string
  password: string
}

export interface InstanceStatus {
  initialized: boolean
  setupKeyRequired: boolean
}

export interface LoginInput {
  email: string
  password: string
  rememberMe?: boolean
}

export interface MessageDetail {
  id: string
  subject: string
  from: string[]
  to: string[]
  cc: string[]
  receivedAt: number
  text: string
  html: string
  attachments: { filename: string | null; contentType: string; size: number }[]
  headers: Record<string, string[]>

  /**
   * True when the message has no HTML part and `html` was generated from its plain text.
   */
  htmlGenerated: boolean
}

/**
 * How well one email client version supports an HTML, CSS or image feature.
 */
export type SupportLevel = 'yes' | 'no' | 'partial' | 'unknown'

/**
 * A client family on one platform, such as Gmail on Android.
 */
export interface EmailClient {
  /**
   * `<family>.<platform>`, as referenced by `ClientSupport.client`.
   */
  id: string
  family: string
  familyName: string
  platform: string
  platformName: string

  /**
   * Whether the client belongs to the families most recipients use; the default filter shows only these.
   */
  main: boolean
}

/**
 * The support of one client version for one feature.
 */
export interface ClientSupport {
  client: string
  version: string

  /**
   * Whether this is the client's most recently tested version for the feature.
   */
  latest: boolean
  support: SupportLevel
  notes: number[]
}

/**
 * One feature detected in a message's HTML, with its support results.
 */
export interface CompatibilityFeature {
  slug: string
  title: string
  category: string
  url: string

  /**
   * Markdown text from caniemail.com.
   */
  description: string

  /**
   * Markdown notes referenced by `ClientSupport.notes`, keyed by number.
   */
  notes: Record<string, string>
  occurrences: number
  examples: string[]

  /**
   * True when every occurrence comes from a selector that could not be checked against the document.
   */
  selectorUnverified: boolean

  /**
   * False for document-structure features that clients remove without breaking the message; they
   * are listed but excluded from the summary and the weighted score.
   */
  scored: boolean

  /**
   * True when clients without support only lose an enhancement, accessibility metadata or a cosmetic
   * detail, including features used only inside `@media` or interaction rules; such features cause
   * differences, never problems.
   */
  graceful: boolean

  /**
   * Percentage points of clients without full support, weighted by the feature's share of elements.
   */
  impact: number
  support: ClientSupport[]
}

/**
 * Mailpit-style weighted coverage in percent: the worst feature's share of clients without full
 * support, weighted by the fraction of elements using it.
 */
export interface WeightedScore {
  supported: number
  partial: number
  unsupported: number
}

/**
 * How clients render the message: with problems when a feature the layout or content depends on is
 * unsupported, with differences when only graceful features are missing or support is partial,
 * identical otherwise; untested without data.
 */
export interface CompatibilitySummary {
  clients: number
  identical: number
  withDifferences: number
  withProblems: number
  untested: number
}

/**
 * Email client compatibility of a message's original HTML, based on caniemail.com data.
 */
export interface HtmlCompatibilityReport {
  /**
   * False when the message has no HTML part; the HTML view then shows markup generated from text.
   */
  htmlPresent: boolean

  /**
   * Linked stylesheets, which are never downloaded and therefore not analysed.
   */
  externalStylesheets: number

  /**
   * Elements inside `<body>`, the denominator of each feature's weight.
   */
  elementCount: number
  data: { source: string; url: string; updated: string; license: string }
  clients: EmailClient[]

  /**
   * Outcomes using each client's latest tested version across all platforms.
   */
  summary: CompatibilitySummary

  /**
   * Weighted coverage over scored features using latest versions across all platforms.
   */
  weighted: WeightedScore

  /**
   * Detected features: scored ones first, by decreasing impact.
   */
  features: CompatibilityFeature[]
}

/**
 * How the SMTP listener protects sessions in transit.
 */
export type ListenerEncryption =
  'none' | 'startTlsOptional' | 'startTlsRequired' | 'implicit'

/**
 * Runtime state of the global SMTP server's listener.
 */
export interface ListenerStatus {
  configuredAddress: string | null
  boundAddress: string | null

  /** Runtime configuration is the sole source of listener settings. */
  source: 'configuration'
  state: 'listening' | 'disabled' | 'failed'
  error: string | null

  /**
   * Whether SMTP sessions must authenticate with an SMTP access before sending mail.
   */
  authenticationRequired: boolean

  /**
   * Whether TLS protects sessions, and how it starts.
   */
  encryption: ListenerEncryption
}

/**
 * The single global SMTP server shared by every SMTP access.
 */
export interface SmtpServer {
  /**
   * Runtime configured `host:port`; null when the server is disabled.
   */
  listenAddress: string | null

  /**
   * Null when no runtime owns the listener (for example, the mock client).
   */
  listener: ListenerStatus | null
}

/**
 * An SMTP credential for external applications. It never carries the password or its hash.
 */

/**
 * An SMTP AUTH mechanism that transmits the password itself.
 */
export type SmtpMechanism = 'plain' | 'login'

/**
 * How one SMTP access may authenticate.
 */
export interface SmtpAccessRules {
  /**
   * Accept the access only on TLS-protected sessions.
   */
  requiresEncryption: boolean

  /**
   * Never empty.
   */
  allowedMechanisms: SmtpMechanism[]
}

export interface SmtpAccess extends SmtpAccessRules {
  id: string
  name: string
  username: string
  enabled: boolean

  /**
   * Milliseconds since the Unix epoch.
   */
  createdAt: number

  /**
   * Milliseconds since the Unix epoch of the latest successful authentication.
   */
  lastUsedAt: number | null
}

export interface SmtpAccessInput extends Partial<SmtpAccessRules> {
  name: string

  /**
   * Omit to let the server derive a unique username from the name.
   */
  username?: string | null
}

/**
 * Credentials returned once, by creation or regeneration. The password cannot be read again;
 * keep this object only in transient component state and discard it when the dialog closes.
 */
export interface IssuedSmtpCredentials {
  access: SmtpAccess
  password: string
  server: SmtpServer
}

/**
 * How mail clients reach the IMAP service: `implicit` is TLS from the first byte (IMAPS), the
 * `startTls*` modes upgrade a plain connection, and `none` sends passwords unencrypted.
 */
export type ImapSecurity =
  'none' | 'startTlsOptional' | 'startTlsRequired' | 'implicit'

/**
 * Connection details of this installation's IMAP service.
 */
export interface ImapServer {
  /**
   * False when the installation runs with the IMAP service disabled.
   */
  enabled: boolean

  /**
   * Host name or address mail clients should connect to.
   */
  host: string | null
  port: number | null
  security: ImapSecurity

  /**
   * The actual bound socket address.
   */
  listenAddress: string | null
}

/**
 * A Smart Folder an IMAP account exposes when it does not mirror every Smart Folder.
 */
export interface ImapLinkedView {
  viewId: string

  /**
   * IMAP mailbox name used instead of the Smart Folder's own name; null follows renames.
   */
  alias: string | null
}

/**
 * A dynamic filter that exists only inside one IMAP account, never in Sandpost's sidebar.
 */
export type ImapLocalFolderKind = 'smart' | 'manual' | 'private'

export interface ImapLocalFolder {
  id: string
  accountId: string
  kind: ImapLocalFolderKind
  definitionId: string | null
  name: string

  /**
   * Canonical query over the owner's readable mail.
   */
  filter: string

  /**
   * Whether its messages also appear in INBOX when the account does not mirror Smart Folders.
   */
  includeInInbox: boolean

  /**
   * Milliseconds since the Unix epoch.
   */
  createdAt: number
  updatedAt: number
}

/**
 * Where an exposed IMAP mailbox takes its messages from.
 */
export type ImapMailboxSource = 'inbox' | 'trash' | 'view' | 'folder'

/**
 * One IMAP mailbox name an account currently exposes, in the order clients list them.
 */
export interface ImapMailbox {
  name: string
  source: ImapMailboxSource
  viewId: string | null
  folderId: string | null
}

/**
 * A personal IMAP account for a mail client. It never carries the password or its hash, and it
 * never sees more mail than its owner can read in Sandpost.
 */
export interface ImapAccount {
  id: string
  ownerId: string
  name: string
  username: string
  enabled: boolean

  /**
   * Mirror exposes current Web folders; Independent keeps account-local folder state.
   */
  mode: ImapAccountMode

  /**
   * Milliseconds since the Unix epoch.
   */
  createdAt: number
  updatedAt: number

  /**
   * Milliseconds since the Unix epoch of the latest successful IMAP login.
   */
  lastUsedAt: number | null
  linkedViews: ImapLinkedView[]
  folders: ImapLocalFolder[]
  mailboxes: ImapMailbox[]
}

export interface ImapAccountInput {
  name: string

  /**
   * Omit to let the server derive a unique username from the name.
   */
  username?: string | null

  /**
   * Defaults to Mirror.
   */
  mode?: ImapAccountMode
}

export type ImapAccountMode = 'mirror' | 'independent'

/**
 * Partial account change.
 */
export interface ImapAccountUpdate {
  name?: string
  enabled?: boolean
  mode?: ImapAccountMode
}

export interface ImapFolderInput {
  name: string
  filter: string

  /**
   * Defaults to true.
   */
  includeInInbox?: boolean
}

/**
 * IMAP credentials returned once, by creation or regeneration. The password cannot be read again;
 * keep this object only in transient component state and discard it when the dialog closes.
 */
export interface IssuedImapCredentials {
  account: ImapAccount
  password: string
  server: ImapServer
}

/**
 * The signed-in user's own IMAP accounts; any role may use these, and other users' accounts are not found.
 */
export interface SandpostImapClient {
  /**
   * Read the global SMTP listener configuration and runtime status.
   */
  server(): Promise<ImapServer>

  /**
   * List the records available through this client method.
   */
  list(): Promise<ImapAccount[]>

  /**
   * Retrieve the record identified by the supplied ID.
   */
  get(id: string): Promise<ImapAccount>

  /**
   * Create a record from the supplied input.
   */
  create(input: ImapAccountInput): Promise<IssuedImapCredentials>

  /**
   * Apply the supplied changes to the selected record.
   */
  update(id: string, changes: ImapAccountUpdate): Promise<ImapAccount>

  /**
   * Captured mail is untouched.
   */
  delete(id: string): Promise<void>

  /**
   * The previous password and every session it opened stop working immediately.
   */
  regeneratePassword(id: string): Promise<IssuedImapCredentials>

  /**
   * Let messages expunged by mail clients reappear; resolves to how many were restored.
   */
  restoreHidden(id: string): Promise<number>
  folders: {
    /**
     * Create a record from the supplied input.
     */
    create(accountId: string, input: ImapFolderInput): Promise<ImapLocalFolder>

    /**
     * Apply the supplied changes to the selected record.
     */
    update(
      accountId: string,
      folderId: string,
      input: ImapFolderInput
    ): Promise<ImapLocalFolder>

    /**
     * Delete the record identified by the supplied ID.
     */
    delete(accountId: string, folderId: string): Promise<void>
  }
}

export interface Scope {
  id: string
  parentId: string | null
  name: string
  description: string | null
  filter: string
  position: number
  policyVersion: number
}

export type ScopeInput = Omit<Scope, 'policyVersion'>

export interface UserAccount {
  id: string
  name: string
  email: string
  role: GlobalRole
  mailAccess: MailAccess
}

export interface NewUserInput {
  name: string
  email: string
  password: string
  role: GlobalRole
  mailAccess: MailAccess
}

export interface UserUpdate {
  name?: string
  password?: string
  role?: GlobalRole
  mailAccess?: MailAccess
}

/**
 * A query compilation error at a zero-based UTF-8 byte offset.
 */
export interface QueryDiagnostic {
  message: string
  position: number
}

export interface QueryValidation {
  valid: boolean
  error: QueryDiagnostic | null
}

export interface ScopePreviewInput {
  parentId: string | null
  filter: string
  limit?: number
}

/**
 * Administration of SMTP access, scopes, and accounts.
 */
export interface SandpostAdminClient {
  /**
   * Global SMTP server and SMTP access credentials; only owners and admins may use these.
   */
  smtp: {
    /**
     * Read the global SMTP listener configuration and runtime status.
     */
    server(): Promise<SmtpServer>

    /**
     * List the records available through this client method.
     */
    list(): Promise<SmtpAccess[]>

    /**
     * Create a record from the supplied input.
     */
    create(input: SmtpAccessInput): Promise<IssuedSmtpCredentials>

    /**
     * Issue a replacement credential and invalidate the previous password and sessions.
     */
    regeneratePassword(id: string): Promise<IssuedSmtpCredentials>

    /**
     * Enable or disable the SMTP access.
     */
    setEnabled(id: string, enabled: boolean): Promise<SmtpAccess>

    /**
     * Replace how the access may authenticate; sessions that no longer comply stop delivering.
     */
    updateRules(id: string, rules: SmtpAccessRules): Promise<SmtpAccess>

    /**
     * Delete the record identified by the supplied ID.
     */
    delete(id: string): Promise<void>
  }
  scopes: {
    /**
     * List the records available through this client method.
     */
    list(): Promise<Scope[]>

    /**
     * Save the supplied  definition.
     */
    save(input: ScopeInput): Promise<Scope>

    /**
     * Delete the record identified by the supplied ID.
     */
    delete(id: string): Promise<void>

    /**
     * Preview messages matching the supplied scope definition.
     */
    preview(input: ScopePreviewInput): Promise<MessageSummary[]>

    /**
     * List user identifiers assigned to the selected scope.
     */
    assignments(scopeId: string): Promise<string[]>

    /**
     * Assign the user to the selected scope.
     */
    assign(scopeId: string, userId: string): Promise<void>

    /**
     * Remove the user from the selected scope.
     */
    unassign(scopeId: string, userId: string): Promise<void>
  }
  users: {
    /**
     * List the records available through this client method.
     */
    list(): Promise<UserAccount[]>

    /**
     * Create a record from the supplied input.
     */
    create(input: NewUserInput): Promise<UserAccount>

    /**
     * Apply the supplied changes to the selected record.
     */
    update(id: string, changes: UserUpdate): Promise<UserAccount>

    /**
     * Delete the record identified by the supplied ID.
     */
    delete(id: string): Promise<void>
  }

  /**
   * Validate a query and return any compiler diagnostic.
   */
  validateQuery(query: string): Promise<QueryValidation>
}

export type SandpostEvent =
  | { type: 'ready' | 'resync' | 'disconnected' }
  | { type: 'message'; id: string; sequence: number }

export interface MessageListOptions {
  before?: number
  limit?: number
  inboxId?: string
  viewId?: string
  folderId?: string
  query?: string
}

export interface SandpostClient {
  /**
   * Read whether initial setup is complete and whether a setup key is required.
   */
  instanceStatus(): Promise<InstanceStatus>

  /**
   * Exchange the setup key for a temporary setup authorization.
   */
  authorizeSetup(input: {
    setupKey: string
  }): Promise<{ authorization: string }>

  /**
   * Create the initial owner using the temporary bootstrap authorization.
   */
  setup(input: SetupInput): Promise<void>

  /**
   * Authenticate the supplied credentials and return the signed-in user.
   */
  login(input: LoginInput): Promise<SessionUser>

  /**
   * Read the current session, returning null when it is no longer authenticated.
   */
  session(): Promise<SessionUser | null>

  /**
   * End the current authenticated session.
   */
  logout(): Promise<void>
  inboxes: {
    /**
     * List the records available through this client method.
     */
    list(): Promise<Inbox[]>

    /**
     * Create a record from the supplied input.
     */
    create(input: InboxInput): Promise<Inbox>
  }

  /**
   * Saved views, presented as Smart Folders; listing returns the caller's personal views and every shared view.
   */
  views: {
    /**
     * List the records available through this client method.
     */
    list(): Promise<SavedView[]>

    /**
     * Save the supplied  definition.
     */
    save(view: SavedView): Promise<SavedView>

    /**
     * Delete the record identified by the supplied ID.
     */
    delete(id: string): Promise<void>
  }

  /**
   * User-owned manual folders, shared by Mirror accounts and snapshotted for Independent state.
   */
  folders: {
    /**
     * List the records available through this client method.
     */
    list(): Promise<WebFolder[]>

    /**
     * Create a record from the supplied input.
     */
    create(input: WebFolderInput): Promise<WebFolder>

    /**
     * Apply the supplied changes to the selected record.
     */
    update(id: string, input: WebFolderInput): Promise<WebFolder>

    /**
     * Delete the record identified by the supplied ID.
     */
    delete(id: string): Promise<void>

    /**
     * Fetch message summaries and convert each wire row to the frontend contract.
     */
    listMessages(
      id: string,
      options?: Omit<MessageListOptions, 'inboxId' | 'viewId' | 'folderId'>
    ): Promise<MessageSummary[]>
  }
  messages: {
    /**
     * `query` is canonical DSL; `viewId` applies a saved view on top of the caller's authorization.
     */
    list(options?: MessageListOptions): Promise<MessageSummary[]>

    /**
     * Search messages and return matching summaries.
     */
    search(
      query: string,
      options?: { before?: number; limit?: number }
    ): Promise<MessageSummary[]>

    /**
     * Set shared read status for up to 256 messages.
     */
    setSeen(identifiers: string[], seen: boolean): Promise<number>

    /**
     * Copy up to 256 messages into a Web folder or Inbox; fails atomically on an existing destination copy.
     */
    copy(
      identifiers: string[],
      folderIdentifier: string | null
    ): Promise<number>

    /**
     * Apply supported primary flags; Deleted remains an IMAP occurrence flag and is immutable here.
     */
    setFlags(input: {
      identifiers: string[]
      operation: MessageFlagOperation
      flags: MessageFlags
    }): Promise<number>

    /**
     * Retrieve the record identified by the supplied ID.
     */
    get(id: string): Promise<MessageDetail>

    /**
     * Perform the get raw operation.
     */
    getRaw(id: string): Promise<Uint8Array>

    /**
     * Check the message's original HTML against email client support data.
     */
    htmlCheck(id: string): Promise<HtmlCompatibilityReport>

    /**
     * Permanently hide one message for the caller; from outside Trash prefer `trash`.
     */
    delete(id: string): Promise<void>

    /**
     * Place up to 256 messages in a manual folder, or back in Inbox when null; this also restores them from Trash.
     */
    move(
      identifiers: string[],
      folderIdentifier: string | null
    ): Promise<number>

    /**
     * Move up to 256 messages to the caller's Trash; resolves to how many authorized messages are now trashed.
     */
    trash(identifiers: string[]): Promise<number>

    /**
     * Perform the get attachment operation.
     */
    getAttachment(id: string, index: number): Promise<Uint8Array>
  }

  /**
   * The caller's Trash: messages deleted from the Web UI that can still be restored or deleted permanently.
   */
  trash: {
    /**
     * Trashed messages newest first; search never includes them, so `query` only narrows this list.
     */
    listMessages(
      options?: Omit<MessageListOptions, 'inboxId' | 'viewId' | 'folderId'>
    ): Promise<MessageSummary[]>

    /**
     * Permanently delete every trashed message; resolves to how many were deleted.
     */
    empty(): Promise<number>
  }
  events: {
    /**
     * Subscribe to runtime events and return a function that removes the subscription.
     */
    subscribe(listener: (event: SandpostEvent) => void): Promise<() => void>
  }

  /**
   * Personal IMAP accounts for mail clients such as Apple Mail and Thunderbird.
   */
  imap: SandpostImapClient
  admin: SandpostAdminClient
}
