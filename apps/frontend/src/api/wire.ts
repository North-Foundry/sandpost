/**
 * Rust mailbox representation used by both HTTP and native transports.
 */
export interface WireMailbox {
  address: string
  domain: string
}

/**
 * Rust message-list representation serialized by serde.
 */
export interface WireMessageSummary {
  id: string
  seq: number
  subject: string
  from: WireMailbox[]
  to: WireMailbox[]
  received_at: number
  size: number
  attachment_count: number
  seen: boolean

  /**
   * Added with primary message flag support; optional for compatibility with older servers.
   */
  flags?: WireMessageFlags
  envelope_to?: WireMailbox[]
  preview?: string
}

export interface WireMessageFlags {
  seen: boolean
  answered: boolean
  flagged: boolean
  draft: boolean
  deleted: boolean
  keywords: string[]
}

/**
 * Inbox representation shared by the HTTP API and native bridge.
 */
export interface WireInbox {
  id: string
  name: string
  address: string | null
  system: boolean
}

/**
 * Rust `View`: a saved query, personal when it has an owner.
 */
export interface WireView {
  identifier: string
  owner_identifier: string | null
  name: string
  filter: string
  icon?: string | null
}

/**
 * Rust WebFolder DTO; timestamps are Unix seconds.
 */
export interface WireWebFolder {
  identifier: string
  owner_identifier: string
  name: string
  icon?: string | null
  exposed_accounts: string[]
  created_at: number
  updated_at: number
}

/**
 * Rust message-detail envelope serialized by the HTTP API and desktop bridge.
 */
export interface WireMessageDetail {
  id: string
  facts: {
    subject: string
    from: WireMailbox[]
    to: WireMailbox[]
    cc: WireMailbox[]
    received_at: number
    text: string
    html: string
    headers: Record<string, string[]>
  }
  attachments: {
    filename: string | null
    content_type: string
    size: number
    content_hash: string
  }[]

  /**
   * True when the message has no `text/html` part, so `facts.html` was generated from its text.
   * Absent from runtimes that predate the flag.
   */
  html_generated?: boolean
}

/**
 * Rust authenticated session identity.
 */
export interface WireSessionUser {
  id: string
  name: string
  email: string
  role: 'owner' | 'member' | 'viewer'
}

/**
 * Rust shared link check state of one message.
 */
export interface WireLinkCheckReport {
  message_id: string
  running: boolean
  started_at: number | null
  checked_at: number | null
  urls: string[]
  omitted_links: number
  summary: {
    total: number
    healthy: number
    broken: number
    server_error: number
    failed: number
    inconclusive: number
  }
  links: {
    url: string
    outcome: 'healthy' | 'broken' | 'server_error' | 'failed' | 'inconclusive'
    status_code: number | null
    status_text: string | null
    final_url: string | null
    redirects: number
    error_kind: string | null
    error_message: string | null
    checked_at: number
  }[]
}

/**
 * Rust result of a bulk link check request.
 */
export interface WireBulkLinkCheckResult {
  started: number
  already_running: number
  skipped: number
}

/**
 * Rust email client compatibility report for a message's original HTML.
 */
export interface WireCompatibilityReport {
  html_present: boolean
  external_stylesheets: number
  element_count: number
  data: { source: string, url: string, updated: string, license: string }
  clients: {
    id: string
    family: string
    family_name: string
    platform: string
    platform_name: string
    main: boolean
  }[]
  summary: { clients: number, identical: number, with_differences: number, with_problems: number, untested: number }
  weighted: { supported: number, partial: number, unsupported: number }
  features: {
    slug: string
    title: string
    category: string
    url: string
    description: string
    notes: Record<string, string>
    occurrences: number
    examples: string[]
    selector_unverified: boolean
    scored: boolean
    graceful: boolean
    impact: number
    support: {
      client: string
      version: string
      latest: boolean
      support: 'yes' | 'no' | 'partial' | 'unknown'
      notes: number[]
    }[]
  }[]
}

/**
 * Rust runtime status of the global SMTP listener.
 */
export interface WireListenerStatus {
  configured_address: string | null
  bound_address: string | null
  source: 'configuration'
  state: 'listening' | 'disabled' | 'failed'
  error: string | null
  authentication_required: boolean

  /**
   * Absent from runtimes that predate SMTP TLS, which never encrypt.
   */
  encryption?: 'none' | 'start_tls_optional' | 'start_tls_required' | 'implicit'
}

/**
 * Rust `SmtpServerDetails`.
 */
export interface WireSmtpServer {
  listen_address: string | null
  listener: WireListenerStatus | null
}

/**
 * Rust `SmtpAccess`; timestamps are Unix seconds.
 */
export interface WireSmtpAccess {
  identifier: string
  name: string
  username: string
  enabled: boolean
  created_at: number
  last_used_at: number | null

  /**
   * Absent from runtimes that predate per-access rules, which accept any connection.
   */
  requires_encryption?: boolean
  allowed_mechanisms?: ('plain' | 'login')[]
}

/**
 * Rust `SmtpAccessUpdate`; omitted fields keep their value.
 */
export interface WireSmtpAccessUpdate {
  enabled?: boolean
  requires_encryption?: boolean
  allowed_mechanisms?: ('plain' | 'login')[]
}

/**
 * Rust `IssuedSmtpCredentials`, the only representation carrying a password.
 */
export interface WireIssuedSmtpCredentials {
  access: WireSmtpAccess
  password: string
  server: WireSmtpServer
}

/**
 * Rust `NewSmtpAccessRequest`.
 */
export interface WireSmtpAccessInput {
  name: string
  username: string | null
  requires_encryption?: boolean
  allowed_mechanisms?: ('plain' | 'login')[]
}

/**
 * Rust `ImapSecurity`.
 */
export type WireImapSecurity =
  'none' | 'start_tls_optional' | 'start_tls_required' | 'implicit'

/**
 * Rust `ImapServerDetails`.
 */
export interface WireImapServer {
  enabled: boolean
  host: string | null
  port: number | null
  security: WireImapSecurity
  listen_address: string | null
}

/**
 * Rust `ImapLinkedView`.
 */
export interface WireImapLinkedView {
  view_identifier: string
  alias: string | null
}

/**
 * Rust `ImapLocalFolder`; timestamps are Unix seconds.
 */
export interface WireImapLocalFolder {
  identifier: string
  account_identifier: string
  kind: 'smart' | 'manual' | 'private'
  definition_identifier: string | null
  name: string
  filter: string
  include_in_inbox: boolean
  created_at: number
  updated_at: number
}

/**
 * Rust `ImapMailboxPreview`.
 */
export interface WireImapMailbox {
  name: string
  source: 'inbox' | 'trash' | 'view' | 'folder'
  view_identifier: string | null
  folder_identifier: string | null
}

/**
 * Rust `ImapAccountDetails`; timestamps are Unix seconds and no password is ever included.
 */
export interface WireImapAccount {
  identifier: string
  owner_identifier: string
  name: string
  username: string
  enabled: boolean
  mode?: 'mirror' | 'independent'

  /**
   * Legacy output accepted from older servers.
   */
  mirror_views?: boolean
  created_at: number
  updated_at: number
  last_used_at: number | null
  linked_views: WireImapLinkedView[]
  folders: WireImapLocalFolder[]
  mailboxes: WireImapMailbox[]
}

/**
 * Rust `IssuedImapCredentials`, the only representation carrying an IMAP password.
 */
export interface WireIssuedImapCredentials {
  account: WireImapAccount
  password: string
  server: WireImapServer
}

/**
 * Rust `NewImapAccountRequest`.
 */
export interface WireImapAccountInput {
  name: string
  username: string | null
  mode: 'mirror' | 'independent'

  /**
   * Legacy request fields accepted by the server for existing clients.
   */
  mirror_views?: boolean
  linked_views?: WireImapLinkedView[]
}

/**
 * Rust `ImapAccountUpdate`; omitted fields keep their value.
 */
export interface WireImapAccountUpdate {
  name?: string
  enabled?: boolean
  mode?: 'mirror' | 'independent'
  mirror_views?: boolean
  linked_views?: WireImapLinkedView[]
}

/**
 * Rust `ImapFolderRequest`.
 */
export interface WireImapFolderInput {
  name: string
  filter: string

  /**
   * Legacy field retained for server compatibility.
   */
  include_in_inbox?: boolean
}

/**
 * Rust `Scope` definition.
 */
export interface WireScope {
  id: string
  parent: string | null
  name: string
  description: string | null
  filter: string
  position: number
  policy_version: number
}

/**
 * Rust account details without credentials.
 */
export interface WireAccount {
  identifier: string
  name: string
  email: string
  role: 'owner' | 'member' | 'viewer'
  mail_access: 'all' | 'scoped'
  personal_filter: string | null
}

/**
 * Rust query validation result.
 */
export interface WireQueryValidation {
  valid: boolean
  error: { message: string; position: number } | null
}

/**
 * Rust outbound request outcome of a Persona step.
 */
export interface WirePersonaHttpRequest {
  url: string
  final_url: string | null
  status_code: number | null
  redirect_count: number
  content_type: string | null
  received_bytes: number
  truncated: boolean
  error_kind: string | null
  error_message: string | null
  duration_milliseconds: number
}

/**
 * Rust Persona step detail, tagged by kind.
 */
export type WirePersonaActionDetail =
  | { kind: 'branch'; condition: string; outcome: boolean }
  | { kind: 'open'; images: WirePersonaHttpRequest[]; omitted_images: number; embedded_images: number; rejected_images: number }
  | { kind: 'wait'; milliseconds: number }
  | { kind: 'click'; selector: string; href: string | null; text: string | null; url: string | null; request: WirePersonaHttpRequest | null }

/**
 * Rust recorded Persona step.
 */
export interface WirePersonaExecutionAction {
  step: number
  status: string
  line: number
  column: number
  started_at: number
  duration_milliseconds: number | null
  error_message: string | null
  detail: WirePersonaActionDetail
}

/**
 * Rust Persona execution with its message subject.
 */
export interface WirePersonaExecution {
  id: string
  seq: number
  persona_id: string
  message_id: string
  status: string
  scheduled_at: number
  started_at: number | null
  completed_at: number | null
  persona_revision: number | null
  error_message: string | null
  subject: string
  actions?: WirePersonaExecutionAction[]
}

/**
 * Rust Persona details.
 */
export interface WirePersona {
  id: string
  owner_id: string
  name: string
  address: string
  script: string
  enabled: boolean
  revision: number
  created_at: number
  updated_at: number
  address_access: string
  last_execution: WirePersonaExecution | null
}

/**
 * Rust PersonaScript validation result.
 */
export interface WirePersonaScriptValidation {
  valid: boolean
  error: { message: string; position: number; line: number; column: number } | null
  statements: number | null
  longest_wait_milliseconds: number | null
}
