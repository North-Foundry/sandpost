import type {
  IssuedSmtpCredentials,
  ListenerEncryption,
  ListenerStatus,
  NewUserInput,
  QueryValidation,
  Scope,
  ScopeInput,
  ScopePreviewInput,
  SmtpAccess,
  SmtpAccessInput,
  SmtpAccessRules,
  SmtpMechanism,
  SmtpServer,
  UserAccount,
  UserUpdate,
} from './contracts'
import type {
  WireAccount,
  WireIssuedSmtpCredentials,
  WireListenerStatus,
  WireQueryValidation,
  WireScope,
  WireSmtpAccess,
  WireSmtpAccessInput,
  WireSmtpAccessUpdate,
  WireSmtpServer,
} from './wire'

/**
 * Number of newest matches a scope preview samples when the caller does not choose.
 */
export const SCOPE_PREVIEW_LIMIT = 20

/**
 * Convert the Rust listener status, keeping its error text verbatim.
 */
function normalizeListener(status: WireListenerStatus): ListenerStatus {
  return {
    configuredAddress: status.configured_address ?? null,
    boundAddress: status.bound_address ?? null,
    source: status.source,
    state: status.state,
    error: status.error ?? null,
    authenticationRequired: Boolean(status.authentication_required),
    encryption: normalizeEncryption(status.encryption),
  }
}

/**
 * Convert the Rust encryption mode; unknown or missing values mean no TLS.
 */
function normalizeEncryption(
  encryption: WireListenerStatus['encryption']
): ListenerEncryption {
  switch (encryption) {
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
 * Convert the Rust global SMTP server details.
 */
export function normalizeSmtpServer(
  representation: WireSmtpServer
): SmtpServer {
  return {
    listenAddress: representation.listen_address ?? null,
    listener: representation.listener
      ? normalizeListener(representation.listener)
      : null,
  }
}

/**
 * Convert a Rust SMTP access, turning Unix seconds into milliseconds.
 */
export function normalizeSmtpAccess(
  representation: WireSmtpAccess
): SmtpAccess {
  return {
    id: String(representation.identifier),
    name: String(representation.name),
    username: String(representation.username),
    enabled: Boolean(representation.enabled),
    createdAt: Number(representation.created_at) * 1000,
    lastUsedAt:
      representation.last_used_at == null
        ? null
        : Number(representation.last_used_at) * 1000,
    requiresEncryption: Boolean(representation.requires_encryption),
    allowedMechanisms: normalizeMechanisms(representation.allowed_mechanisms),
  }
}

/**
 * Every SMTP AUTH mechanism, in the order the server reports them.
 */
export const SMTP_MECHANISMS: readonly SmtpMechanism[] = ['plain', 'login']

/**
 * Keep known mechanisms in canonical order; a missing list means every mechanism is allowed.
 */
function normalizeMechanisms(
  mechanisms: WireSmtpAccess['allowed_mechanisms']
): SmtpMechanism[] {
  if (!mechanisms) return [...SMTP_MECHANISMS]
  return SMTP_MECHANISMS.filter((mechanism) => mechanisms.includes(mechanism))
}

/**
 * Encode replacement authentication rules for an access.
 */
export function smtpAccessRulesWire(
  rules: SmtpAccessRules
): WireSmtpAccessUpdate {
  return {
    requires_encryption: rules.requiresEncryption,
    allowed_mechanisms: [...rules.allowedMechanisms],
  }
}

/**
 * Convert issued credentials; the password is passed through untouched and never stored.
 */
export function normalizeIssuedCredentials(
  representation: WireIssuedSmtpCredentials
): IssuedSmtpCredentials {
  return {
    access: normalizeSmtpAccess(representation.access),
    password: String(representation.password),
    server: normalizeSmtpServer(representation.server),
  }
}

/**
 * Encode a new SMTP access; blank optional fields let the server choose.
 */
export function smtpAccessWire(input: SmtpAccessInput): WireSmtpAccessInput {
  return {
    name: input.name,
    username: input.username?.trim() || null,
    ...(input.requiresEncryption === undefined
      ? {}
      : { requires_encryption: input.requiresEncryption }),
    ...(input.allowedMechanisms === undefined
      ? {}
      : { allowed_mechanisms: [...input.allowedMechanisms] }),
  }
}

/**
 * Convert a Rust scope definition to the frontend contract.
 */
export function normalizeScope(representation: WireScope): Scope {
  return {
    id: String(representation.id),
    parentId: representation.parent ?? null,
    name: String(representation.name),
    description: representation.description ?? null,
    filter: String(representation.filter ?? ''),
    position: Number(representation.position),
    policyVersion: Number(representation.policy_version),
  }
}

/**
 * Encode a scope edit; the server assigns the policy version.
 */
export function scopeWire(input: ScopeInput): WireScope {
  return {
    id: input.id,
    parent: input.parentId,
    name: input.name,
    description: input.description,
    filter: input.filter,
    position: input.position,
    policy_version: 0,
  }
}

/**
 * Encode a scope preview request with the default sample size.
 */
export function scopePreviewWire(input: ScopePreviewInput) {
  return {
    parent: input.parentId,
    filter: input.filter,
    limit: input.limit ?? SCOPE_PREVIEW_LIMIT,
  }
}

/**
 * Convert Rust account details to the frontend contract.
 */
export function normalizeAccount(representation: WireAccount): UserAccount {
  return {
    id: String(representation.identifier),
    name: String(representation.name),
    email: String(representation.email),
    role: representation.role,
    mailAccess: representation.mail_access === 'all' ? 'all' : 'scoped',
  }
}

/**
 * Encode a new account with Rust field names.
 */
export function newAccountWire(input: NewUserInput) {
  return {
    name: input.name,
    email: input.email,
    password: input.password,
    role: input.role,
    mail_access: input.mailAccess,
  }
}

/**
 * Encode an account update, sending only the fields that change.
 */
export function accountUpdateWire(changes: UserUpdate) {
  return {
    ...(changes.name !== undefined ? { name: changes.name } : {}),
    ...(changes.password !== undefined ? { password: changes.password } : {}),
    ...(changes.role !== undefined ? { role: changes.role } : {}),
    ...(changes.mailAccess !== undefined
      ? { mail_access: changes.mailAccess }
      : {}),
  }
}

/**
 * Convert a Rust query validation result to the frontend contract.
 */
export function normalizeValidation(
  representation: WireQueryValidation
): QueryValidation {
  return {
    valid: Boolean(representation.valid),
    error: representation.error
      ? {
        message: String(representation.error.message),
        position: Number(representation.error.position),
      }
      : null,
  }
}

/**
 * Translate a UTF-8 byte offset reported by the query compiler into a JavaScript string index.
 */
export function characterIndexForByteOffset(
  source: string,
  byteOffset: number
): number {
  let bytes = 0
  let index = 0
  for (const character of source) {
    if (bytes >= byteOffset) return index
    bytes += new TextEncoder().encode(character).length
    index += character.length
  }
  return source.length
}
