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
 * A null owner marks a shared view managed by instance owners.
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

export type GlobalRole = 'owner' | 'member' | 'viewer'

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
 * Verdict of one verified link: a 2xx response, a 4xx client error, a 5xx server error, no
 * response at all, or a response or policy that prevents a verdict.
 */
export type LinkOutcome = 'healthy' | 'broken' | 'server_error' | 'failed' | 'inconclusive'

/**
 * Why a link check produced no usable response.
 */
export type LinkCheckErrorKind =
  | 'name_resolution'
  | 'connection'
  | 'tls'
  | 'timeout'
  | 'protocol'
  | 'too_many_redirects'
  | 'redirect_loop'
  | 'invalid_redirect'
  | 'blocked_address'

/**
 * The stored verification result of one distinct link of a message.
 */
export interface LinkCheckResult {
  /**
   * Absolute URL as found in the message, without its fragment.
   */
  url: string
  outcome: LinkOutcome
  statusCode: number | null

  /**
   * Registered reason phrase of the status code, such as `Not Found`.
   */
  statusText: string | null

  /**
   * The URL that produced the final response, present only after redirects.
   */
  finalUrl: string | null
  redirects: number
  errorKind: LinkCheckErrorKind | null
  errorMessage: string | null

  /**
   * Unix seconds when this link's verification finished.
   */
  checkedAt: number
}

/**
 * Counts of a message's stored link results by outcome.
 */
export interface LinkCheckSummary {
  total: number
  healthy: number
  broken: number
  serverError: number
  failed: number
  inconclusive: number
}

/**
 * The shared link check state of one message, identical for every reader who can open it.
 */
export interface LinkCheckReport {
  messageId: string

  /**
   * A check is queued or running; the stored results remain until it completes.
   */
  running: boolean
  startedAt: number | null

  /**
   * Unix seconds when the stored check finished; null before the first check.
   */
  checkedAt: number | null

  /**
   * Distinct links found in the message, in first-appearance order, whether checked or not.
   */
  urls: string[]

  /**
   * Distinct links beyond the per-message limit, which are neither listed nor verified.
   */
  omittedLinks: number
  summary: LinkCheckSummary
  links: LinkCheckResult[]
}

/**
 * What a bulk link check request did with the selected messages.
 */
export interface BulkLinkCheckResult {
  started: number
  alreadyRunning: number
  skipped: number
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
 * How an SMTP Outbound protects the connection: plain text, upgraded with STARTTLS, or implicit TLS.
 */
export type OutboundSecurity = 'none' | 'starttls' | 'tls'

/**
 * How an SMTP Outbound authenticates to the external server.
 */
export type OutboundAuthenticationType = 'none' | 'plain' | 'login'

/**
 * Category of an outbound connection or delivery failure.
 */
export type OutboundErrorKind =
  | 'configuration'
  | 'policy'
  | 'connection'
  | 'tls'
  | 'authentication'
  | 'temporary'
  | 'permanent'
  | 'interrupted'

/**
 * An outbound failure; the message keeps the server's original text, including any SMTP reply.
 */
export interface OutboundError {
  kind: OutboundErrorKind
  message: string

  /**
   * SMTP reply code when the failure came from an SMTP reply.
   */
  code: number | null
}

/**
 * Connection settings shared by stored SMTP Outbounds and their edit input.
 */
export interface SmtpOutboundSettings {
  name: string
  host: string
  port: number
  security: OutboundSecurity
  authenticationType: OutboundAuthenticationType
  username: string | null

  /**
   * Replaces the message From header; null keeps the original.
   */
  overrideFrom: string | null

  /**
   * Replaces the SMTP envelope sender; null keeps the original.
   */
  envelopeFrom: string | null

  /**
   * Seconds; null uses the server default of 10.
   */
  connectTimeoutSeconds: number | null

  /**
   * Seconds; null uses the server default of 60.
   */
  smtpTimeoutSeconds: number | null
  tlsVerify: boolean
  heloName: string | null

  /**
   * Null uses the server default of 2.
   */
  maxConnections: number | null

  /**
   * Recipient safety allowlist of glob patterns. Empty permits no recipient; `*` permits everyone.
   */
  allowedRecipients: string[]
}

/**
 * An instance-level connection to an external SMTP server. The password is never returned.
 */
export interface SmtpOutbound extends SmtpOutboundSettings {
  id: string
  enabled: boolean

  /**
   * Whether a password is stored for this outbound.
   */
  passwordConfigured: boolean

  /**
   * Number of scope routing actions that deliver through this outbound.
   */
  routingActionCount: number

  /**
   * Milliseconds since the Unix epoch.
   */
  createdAt: number

  /**
   * Milliseconds since the Unix epoch.
   */
  updatedAt: number
}

/**
 * Create or replace an SMTP Outbound, or test a draft.
 */
export interface SmtpOutboundInput extends SmtpOutboundSettings {
  /**
   * Write-only; omit, null or blank keeps the stored password on update.
   */
  password?: string | null

  /**
   * Omit to keep the current state on update; creation defaults to disabled.
   */
  enabled?: boolean
}

/**
 * Instance-wide limits for outbound delivery.
 */
export interface OutboundPolicy {
  allowInternalAddresses: boolean
  maximumAttempts: number
  retryDelaySeconds: number

  /**
   * False when the instance has no secret key, so passwords cannot be saved.
   */
  credentialsAvailable: boolean
}

/**
 * Result of connecting, negotiating TLS and authenticating to an external server without sending mail.
 */
export interface OutboundConnectionTest {
  success: boolean
  address: string | null
  encrypted: boolean
  authenticated: boolean
  extensions: string[]
  durationMilliseconds: number
  error: OutboundError | null
}

/**
 * Relay delivers to the original envelope recipients; forward delivers a copy to configured recipients.
 */
export type RoutingActionType = 'relay' | 'forward'

/**
 * A delivery action of one scope through an SMTP Outbound; never inherited by child scopes.
 */
export interface RoutingAction {
  id: string
  scopeId: string
  smtpOutboundId: string
  actionType: RoutingActionType

  /**
   * Empty for relay actions.
   */
  forwardRecipients: string[]
  enabled: boolean

  /**
   * Milliseconds since the Unix epoch.
   */
  createdAt: number

  /**
   * Milliseconds since the Unix epoch.
   */
  updatedAt: number
}

export interface RoutingActionInput {
  smtpOutboundId: string
  actionType: RoutingActionType

  /**
   * Required (1 to 50) for forward actions; ignored for relay actions.
   */
  forwardRecipients: string[]
  enabled?: boolean
}

export type OutboundDeliveryStatus = 'pending' | 'processing' | 'retrying' | 'delivered' | 'failed'

export type OutboundRecipientStatus = 'pending' | 'delivered' | 'failed'

export interface OutboundDeliveryRecipient {
  address: string
  status: OutboundRecipientStatus
  error: OutboundError | null

  /**
   * The server's accepting reply, for example `250 2.0.0 queued`.
   */
  response: string | null

  /**
   * Milliseconds since the Unix epoch.
   */
  deliveredAt: number | null
}

/**
 * A durable background delivery of one message through one routing action. Times are milliseconds.
 */
export interface OutboundDelivery {
  id: string

  /**
   * Keyset cursor for listing older deliveries.
   */
  sequence: number
  messageId: string
  subject: string
  scopeId: string | null
  routingActionId: string | null
  smtpOutboundId: string | null
  smtpOutboundName: string
  actionType: RoutingActionType
  status: OutboundDeliveryStatus
  envelopeFrom: string | null
  attemptCount: number
  lastError: OutboundError | null
  createdAt: number
  lastAttemptAt: number | null
  nextAttemptAt: number | null
  deliveredAt: number | null
  recipients: OutboundDeliveryRecipient[]
}

export type OutboundAttemptOutcome = 'delivered' | 'partial' | 'deferred' | 'failed' | 'interrupted'

/**
 * One connection attempt of a delivery. Times are milliseconds.
 */
export interface OutboundDeliveryAttempt {
  number: number
  startedAt: number
  finishedAt: number | null
  outcome: OutboundAttemptOutcome
  error: OutboundError | null
  deliveredRecipients: number
  deferredRecipients: number
  failedRecipients: number
}

export interface OutboundDeliveryDetail extends OutboundDelivery {
  attempts: OutboundDeliveryAttempt[]
}

export interface OutboundDeliveryListOptions {
  status?: OutboundDeliveryStatus

  /**
   * Exclusive delivery sequence; returns older deliveries.
   */
  before?: number

  /**
   * 1 to 100; the server defaults to 50.
   */
  limit?: number
}

/**
 * Administration of SMTP access, scopes, and accounts.
 */
export interface SandpostAdminClient {
  /**
   * Global SMTP server and SMTP access credentials; only owners may use these.
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
  /**
   * SMTP Outbound connections to external servers; only owners may use these.
   */
  outbound: {
    /**
     * Read the instance-wide outbound delivery policy.
     */
    policy(): Promise<OutboundPolicy>

    /**
     * List SMTP Outbounds ordered by name.
     */
    list(): Promise<SmtpOutbound[]>

    /**
     * Read one SMTP Outbound.
     */
    get(id: string): Promise<SmtpOutbound>

    /**
     * Create an SMTP Outbound; a duplicate name is a conflict.
     */
    create(input: SmtpOutboundInput): Promise<SmtpOutbound>

    /**
     * Replace an SMTP Outbound; a blank password keeps the stored one.
     */
    update(id: string, input: SmtpOutboundInput): Promise<SmtpOutbound>

    /**
     * Enable or disable an SMTP Outbound.
     */
    setEnabled(id: string, enabled: boolean): Promise<SmtpOutbound>

    /**
     * Delete an SMTP Outbound; refused while routing actions reference it.
     */
    delete(id: string): Promise<void>

    /**
     * Test the stored configuration of an SMTP Outbound without sending mail.
     */
    test(id: string): Promise<OutboundConnectionTest>

    /**
     * Test unsaved settings; with `id`, an omitted password reuses that outbound's stored password.
     */
    testDraft(input: SmtpOutboundInput, id?: string | null): Promise<OutboundConnectionTest>
  }

  /**
   * Scope routing actions; only owners may use these.
   */
  routing: {
    /**
     * List the routing actions of one scope.
     */
    list(scopeId: string): Promise<RoutingAction[]>

    /**
     * Add a routing action to a scope.
     */
    create(scopeId: string, input: RoutingActionInput): Promise<RoutingAction>

    /**
     * Replace a routing action.
     */
    update(id: string, input: RoutingActionInput): Promise<RoutingAction>

    /**
     * Delete a routing action.
     */
    delete(id: string): Promise<void>
  }

  /**
   * Outbound delivery jobs; only owners may use these.
   */
  deliveries: {
    /**
     * List deliveries newest first, optionally by status and before a sequence.
     */
    list(options?: OutboundDeliveryListOptions): Promise<OutboundDelivery[]>

    /**
     * Read one delivery with its attempt history.
     */
    get(id: string): Promise<OutboundDeliveryDetail>

    /**
     * Schedule a failed delivery again.
     */
    retry(id: string): Promise<OutboundDelivery>

    /**
     * List the deliveries of one readable message.
     */
    forMessage(messageId: string): Promise<OutboundDelivery[]>
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

/**
 * Whether a Persona owner's current access covers mail addressed to the Persona: every message (`confirmed`),
 * decided message by message (`perMessage`), or none (`denied`).
 */
export type PersonaAddressAccess = 'confirmed' | 'perMessage' | 'denied'

export type PersonaExecutionStatus = 'pending' | 'running' | 'completed' | 'failed' | 'cancelled'

export type PersonaActionStatus = 'started' | 'succeeded' | 'failed' | 'interrupted' | 'cancelled'

/**
 * A user-owned virtual recipient that runs a PersonaScript on mail addressed to it.
 *
 * Personas only act on mail their owner can read; they never grant access. Timestamps are Unix seconds.
 */
export interface Persona {
  id: string
  ownerId: string
  name: string
  address: string
  script: string
  enabled: boolean
  revision: number
  createdAt: number
  updatedAt: number
  addressAccess: PersonaAddressAccess

  /**
   * The newest execution whose message the reader can currently read.
   */
  lastExecution: PersonaExecutionSummary | null
}

export interface PersonaInput {
  name: string
  address: string
  script: string
  enabled: boolean
}

/**
 * One run of a Persona's script for one message. Timestamps are Unix seconds.
 */
export interface PersonaExecutionSummary {
  id: string
  sequence: number
  personaId: string
  messageId: string
  status: PersonaExecutionStatus
  scheduledAt: number
  startedAt: number | null
  completedAt: number | null
  error: string | null
  subject: string
}

/**
 * The outcome of one outbound request made by a Persona.
 */
export interface PersonaHttpRequest {
  url: string
  finalUrl: string | null
  statusCode: number | null
  redirectCount: number
  contentType: string | null
  receivedBytes: number
  truncated: boolean
  errorKind: LinkCheckErrorKind | null
  error: string | null
  durationMilliseconds: number
}

export type PersonaActionDetail =
  | { kind: 'branch'; condition: string; outcome: boolean }
  | { kind: 'open'; images: PersonaHttpRequest[]; omittedImages: number; embeddedImages: number; rejectedImages: number }
  | { kind: 'wait'; milliseconds: number }
  | { kind: 'click'; selector: string; href: string | null; text: string | null; url: string | null; request: PersonaHttpRequest | null }

/**
 * One recorded step of an execution.
 */
export interface PersonaExecutionAction {
  step: number
  status: PersonaActionStatus
  line: number
  column: number
  startedAt: number
  durationMilliseconds: number | null
  error: string | null
  detail: PersonaActionDetail
}

export interface PersonaExecutionDetails extends PersonaExecutionSummary {
  actions: PersonaExecutionAction[]
}

/**
 * The first problem in a script; `position` is a UTF-8 byte offset.
 */
export interface PersonaScriptDiagnostic {
  message: string
  position: number
  line: number
  column: number
}

export interface PersonaScriptValidation {
  valid: boolean
  error: PersonaScriptDiagnostic | null
  statements: number | null
  longestWaitMilliseconds: number | null
}

/**
 * Persona management, the Persona mail view, and execution history.
 */
export interface SandpostPersonaClient {
  /**
   * List the caller's own Personas.
   */
  list(): Promise<Persona[]>

  /**
   * Read one Persona.
   */
  get(id: string): Promise<Persona>

  /**
   * Create a Persona owned by the caller.
   */
  create(input: PersonaInput): Promise<Persona>

  /**
   * Replace the caller's own Persona definition.
   */
  update(id: string, input: PersonaInput): Promise<Persona>

  /**
   * Enable or disable a Persona.
   */
  setEnabled(id: string, enabled: boolean): Promise<Persona>

  /**
   * Delete a Persona and its history; its mail is kept.
   */
  delete(id: string): Promise<void>

  /**
   * Validate PersonaScript without saving it.
   */
  validate(script: string): Promise<PersonaScriptValidation>

  /**
   * List readable mail addressed to a Persona, newest first.
   */
  listMessages(id: string, options?: { before?: number; limit?: number; query?: string }): Promise<MessageSummary[]>

  /**
   * List a Persona's executions whose messages the caller can read, newest first.
   */
  executions(id: string, options?: { before?: number; limit?: number }): Promise<PersonaExecutionSummary[]>

  /**
   * Read one execution with its recorded steps.
   */
  execution(id: string, executionId: string): Promise<PersonaExecutionDetails>
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
     * Read the message's stored link check without starting one.
     */
    linkCheck(id: string): Promise<LinkCheckReport>

    /**
     * Start, or join, a background check of the message's links and return its state.
     */
    startLinkCheck(id: string): Promise<LinkCheckReport>

    /**
     * Start background link checks of up to 256 messages.
     */
    checkLinks(identifiers: string[]): Promise<BulkLinkCheckResult>

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

  /**
   * Personas; absent when the transport does not offer them, such as an older desktop preload.
   */
  personas?: SandpostPersonaClient
  admin: SandpostAdminClient
}
