import type {
  OutboundAttemptOutcome,
  OutboundConnectionTest,
  OutboundDelivery,
  OutboundDeliveryAttempt,
  OutboundDeliveryDetail,
  OutboundDeliveryListOptions,
  OutboundDeliveryRecipient,
  OutboundDeliveryStatus,
  OutboundError,
  OutboundErrorKind,
  OutboundPolicy,
  OutboundRecipientStatus,
  OutboundSecurity,
  OutboundAuthenticationType,
  RoutingAction,
  RoutingActionInput,
  SmtpOutbound,
  SmtpOutboundInput,
} from './contracts'
import type {
  WireConnectionTestResult,
  WireOutboundDelivery,
  WireOutboundDeliveryAttempt,
  WireOutboundDeliveryDetail,
  WireOutboundDeliveryListOptions,
  WireOutboundDeliveryRecipient,
  WireOutboundError,
  WireOutboundPolicy,
  WireRoutingAction,
  WireRoutingActionInput,
  WireSmtpOutbound,
  WireSmtpOutboundInput,
  WireSmtpOutboundTestInput,
} from './wire'

const ERROR_KINDS: readonly OutboundErrorKind[] = [
  'configuration', 'policy', 'connection', 'tls', 'authentication', 'temporary', 'permanent', 'interrupted',
]
const DELIVERY_STATUSES: readonly OutboundDeliveryStatus[] = ['pending', 'processing', 'retrying', 'delivered', 'failed']
const RECIPIENT_STATUSES: readonly OutboundRecipientStatus[] = ['pending', 'delivered', 'failed']
const ATTEMPT_OUTCOMES: readonly OutboundAttemptOutcome[] = ['delivered', 'partial', 'deferred', 'failed', 'interrupted']

/**
 * Convert Unix seconds to milliseconds, keeping null for absent times.
 */
function milliseconds(seconds: number | null | undefined): number | null {
  return seconds == null ? null : Number(seconds) * 1000
}

/**
 * Trim an optional text field, sending null when it is blank so the server uses its default.
 */
function optionalText(value: string | null | undefined): string | null {
  return value?.trim() || null
}

/**
 * Convert a Rust outbound error; unknown kinds become permanent so they never look recoverable.
 */
export function normalizeOutboundError(representation: WireOutboundError | null | undefined): OutboundError | null {
  if (!representation) return null
  return {
    kind: ERROR_KINDS.find((kind) => kind === representation.kind) ?? 'permanent',
    message: String(representation.message ?? ''),
    code: representation.code == null ? null : Number(representation.code),
  }
}

/**
 * Convert a Rust SMTP Outbound, turning Unix seconds into milliseconds. Any stray password is dropped.
 */
export function normalizeSmtpOutbound(representation: WireSmtpOutbound): SmtpOutbound {
  return {
    id: String(representation.id),
    name: String(representation.name),
    host: String(representation.host),
    port: Number(representation.port),
    security: normalizeSecurity(representation.security),
    authenticationType: normalizeAuthenticationType(representation.auth_type),
    username: representation.username ?? null,
    passwordConfigured: Boolean(representation.password_configured),
    enabled: Boolean(representation.enabled),
    overrideFrom: representation.override_from ?? null,
    envelopeFrom: representation.envelope_from ?? null,
    connectTimeoutSeconds: representation.connect_timeout ?? null,
    smtpTimeoutSeconds: representation.smtp_timeout ?? null,
    tlsVerify: representation.tls_verify !== false,
    heloName: representation.helo_name ?? null,
    maxConnections: representation.max_connections ?? null,
    allowedRecipients: [...(representation.allowed_recipients ?? [])].map(String),
    routingActionCount: Number(representation.routing_action_count ?? 0),
    createdAt: Number(representation.created_at) * 1000,
    updatedAt: Number(representation.updated_at) * 1000,
  }
}

/**
 * Convert the Rust security mode; unknown values are reported as plain text, the weakest assumption.
 */
function normalizeSecurity(security: string): OutboundSecurity {
  return security === 'starttls' || security === 'tls' ? security : 'none'
}

/**
 * Convert the Rust authentication type; unknown values mean no authentication.
 */
function normalizeAuthenticationType(authenticationType: string): OutboundAuthenticationType {
  return authenticationType === 'plain' || authenticationType === 'login' ? authenticationType : 'none'
}

/**
 * Encode an SMTP Outbound for create, replace or draft test.
 *
 * Blank optional text becomes null. The password is sent only when the outbound authenticates and a
 * non-empty value was typed, so leaving it blank keeps the stored password. `enabled` is sent only when
 * chosen, and the recipient allowlist is always sent, including an empty list that permits no recipient.
 */
export function smtpOutboundWire(input: SmtpOutboundInput): WireSmtpOutboundInput {
  const authenticates = input.authenticationType !== 'none'
  return {
    name: input.name.trim(),
    host: input.host.trim(),
    port: input.port,
    security: input.security,
    auth_type: input.authenticationType,
    username: authenticates ? optionalText(input.username) : null,
    ...(authenticates && input.password ? { password: input.password } : {}),
    ...(input.enabled === undefined ? {} : { enabled: input.enabled }),
    override_from: optionalText(input.overrideFrom),
    envelope_from: optionalText(input.envelopeFrom),
    connect_timeout: input.connectTimeoutSeconds,
    smtp_timeout: input.smtpTimeoutSeconds,
    tls_verify: input.tlsVerify,
    helo_name: optionalText(input.heloName),
    max_connections: input.maxConnections,
    allowed_recipients: [...input.allowedRecipients],
  }
}

/**
 * Encode a draft connection test; `id` lets the server reuse that outbound's stored password.
 */
export function smtpOutboundTestWire(input: SmtpOutboundInput, id?: string | null): WireSmtpOutboundTestInput {
  return { ...smtpOutboundWire(input), ...(id ? { id } : {}) }
}

/**
 * Convert the Rust outbound policy.
 */
export function normalizeOutboundPolicy(representation: WireOutboundPolicy): OutboundPolicy {
  return {
    allowInternalAddresses: Boolean(representation.allow_internal_addresses),
    maximumAttempts: Number(representation.maximum_attempts),
    retryDelaySeconds: Number(representation.retry_delay_seconds),
    credentialsAvailable: representation.credentials_available !== false,
  }
}

/**
 * Convert a Rust connection test result.
 */
export function normalizeConnectionTest(representation: WireConnectionTestResult): OutboundConnectionTest {
  return {
    success: Boolean(representation.success),
    address: representation.address ?? null,
    encrypted: Boolean(representation.encrypted),
    authenticated: Boolean(representation.authenticated),
    extensions: [...(representation.extensions ?? [])].map(String),
    durationMilliseconds: Number(representation.duration_milliseconds ?? 0),
    error: normalizeOutboundError(representation.error),
  }
}

/**
 * Convert a Rust routing action, turning Unix seconds into milliseconds.
 */
export function normalizeRoutingAction(representation: WireRoutingAction): RoutingAction {
  return {
    id: String(representation.id),
    scopeId: String(representation.scope_id),
    smtpOutboundId: String(representation.smtp_outbound_id),
    actionType: representation.action_type === 'forward' ? 'forward' : 'relay',
    forwardRecipients: [...(representation.forward_recipients ?? [])].map(String),
    enabled: representation.enabled !== false,
    createdAt: Number(representation.created_at) * 1000,
    updatedAt: Number(representation.updated_at) * 1000,
  }
}

/**
 * Encode a routing action; relay actions never carry forward recipients.
 */
export function routingActionWire(input: RoutingActionInput): WireRoutingActionInput {
  return {
    smtp_outbound_id: input.smtpOutboundId,
    action_type: input.actionType,
    forward_recipients: input.actionType === 'forward' ? input.forwardRecipients.map((address) => address.trim()).filter(Boolean) : [],
    ...(input.enabled === undefined ? {} : { enabled: input.enabled }),
  }
}

/**
 * Convert one recipient's delivery state.
 */
function normalizeRecipient(representation: WireOutboundDeliveryRecipient): OutboundDeliveryRecipient {
  return {
    address: String(representation.address),
    status: RECIPIENT_STATUSES.find((status) => status === representation.status) ?? 'failed',
    error: normalizeOutboundError(representation.error),
    response: representation.response ?? null,
    deliveredAt: milliseconds(representation.delivered_at),
  }
}

/**
 * Convert a Rust delivery; unknown statuses become failed so they never look delivered.
 */
export function normalizeOutboundDelivery(representation: WireOutboundDelivery): OutboundDelivery {
  return {
    id: String(representation.id),
    sequence: Number(representation.seq),
    messageId: String(representation.message_id),
    subject: String(representation.subject ?? ''),
    scopeId: representation.scope_id ?? null,
    routingActionId: representation.routing_action_id ?? null,
    smtpOutboundId: representation.smtp_outbound_id ?? null,
    smtpOutboundName: String(representation.smtp_outbound_name ?? ''),
    actionType: representation.action_type === 'forward' ? 'forward' : 'relay',
    status: DELIVERY_STATUSES.find((status) => status === representation.status) ?? 'failed',
    envelopeFrom: representation.envelope_from ?? null,
    attemptCount: Number(representation.attempt_count ?? 0),
    lastError: normalizeOutboundError(representation.last_error),
    createdAt: Number(representation.created_at) * 1000,
    lastAttemptAt: milliseconds(representation.last_attempt_at),
    nextAttemptAt: milliseconds(representation.next_attempt_at),
    deliveredAt: milliseconds(representation.delivered_at),
    recipients: (representation.recipients ?? []).map(normalizeRecipient),
  }
}

/**
 * Convert one delivery attempt; unknown outcomes become failed.
 */
function normalizeAttempt(representation: WireOutboundDeliveryAttempt): OutboundDeliveryAttempt {
  return {
    number: Number(representation.number),
    startedAt: Number(representation.started_at) * 1000,
    finishedAt: milliseconds(representation.finished_at),
    outcome: ATTEMPT_OUTCOMES.find((outcome) => outcome === representation.outcome) ?? 'failed',
    error: normalizeOutboundError(representation.error),
    deliveredRecipients: Number(representation.delivered_recipients ?? 0),
    deferredRecipients: Number(representation.deferred_recipients ?? 0),
    failedRecipients: Number(representation.failed_recipients ?? 0),
  }
}

/**
 * Convert a Rust delivery with its attempt history.
 */
export function normalizeOutboundDeliveryDetail(representation: WireOutboundDeliveryDetail): OutboundDeliveryDetail {
  return {
    ...normalizeOutboundDelivery(representation),
    attempts: (representation.attempts ?? []).map(normalizeAttempt),
  }
}

/**
 * Encode delivery list filters, leaving out unset ones.
 */
export function outboundDeliveryListWire(options: OutboundDeliveryListOptions = {}): WireOutboundDeliveryListOptions {
  return {
    ...(options.status === undefined ? {} : { status: options.status }),
    ...(options.before === undefined ? {} : { before: options.before }),
    ...(options.limit === undefined ? {} : { limit: options.limit }),
  }
}

/**
 * Encode delivery list filters as a query string, empty when nothing is set.
 */
export function outboundDeliveryListQuery(options: OutboundDeliveryListOptions = {}): string {
  const query = new URLSearchParams()
  for (const [name, value] of Object.entries(outboundDeliveryListWire(options))) query.set(name, String(value))
  return query.size ? `?${query}` : ''
}
