/**
 * Editable state, validation and encoding of the SMTP Outbound dialog.
 */
import type { OutboundAuthenticationType, OutboundSecurity, SmtpOutbound, SmtpOutboundInput } from '@/api/contracts'
import {
  MAXIMUM_ALLOWED_RECIPIENTS,
  allowlistCoverage,
  formatRecipientList,
  isValidRecipientAddress,
  isValidRecipientPattern,
  parseRecipientList,
} from '@/features/outbound/recipients'

/**
 * Text-backed form values; numbers stay strings until validation so partial input is kept.
 */
export interface OutboundForm {
  name: string
  host: string
  port: string
  security: OutboundSecurity
  authenticationType: OutboundAuthenticationType
  username: string

  /**
   * Write-only; blank keeps the stored password when editing.
   */
  password: string
  enabled: boolean
  overrideFrom: string
  envelopeFrom: string
  connectTimeout: string
  smtpTimeout: string
  tlsVerify: boolean
  heloName: string
  maxConnections: string

  /**
   * Allowlist patterns as typed, one per line.
   */
  allowedRecipients: string
}

export type OutboundFormField =
  | 'name' | 'host' | 'port' | 'username' | 'password' | 'overrideFrom' | 'envelopeFrom'
  | 'connectTimeout' | 'smtpTimeout' | 'heloName' | 'maxConnections' | 'allowedRecipients'

/**
 * A validation problem as an i18n key with its named parameters.
 */
export interface OutboundFormProblem {
  key: string
  params?: Record<string, string | number>
}

/**
 * Situations the dialog warns about without blocking the save.
 */
export interface OutboundFormWarnings {
  /**
   * Credentials would cross the network unencrypted.
   */
  plaintextCredentials: boolean

  /**
   * TLS is used but the server certificate is not verified.
   */
  certificateUnverified: boolean

  /**
   * The empty allowlist delivers to nobody.
   */
  nobodyAllowed: boolean

  /**
   * The allowlist delivers to every recipient.
   */
  everyoneAllowed: boolean
}

/**
 * Conventional port of each security mode.
 */
export const DEFAULT_PORTS: Record<OutboundSecurity, number> = { none: 25, starttls: 587, tls: 465 }

/**
 * Server defaults applied when an advanced value is left blank.
 */
export const OUTBOUND_DEFAULTS = { connectTimeout: 10, smtpTimeout: 60, maxConnections: 2 } as const

/**
 * Bounds of each numeric field, inclusive.
 */
const NUMBER_BOUNDS: Record<'port' | 'connectTimeout' | 'smtpTimeout' | 'maxConnections', readonly [number, number]> = {
  port: [1, 65535],
  connectTimeout: [1, 300],
  smtpTimeout: [1, 600],
  maxConnections: [1, 16],
}

/**
 * A host name or IP literal without scheme, path, port or spaces.
 */
const HOST = /^(?:\[[0-9A-Fa-f:.]+\]|[A-Za-z0-9](?:[A-Za-z0-9.-]*[A-Za-z0-9])?|[0-9A-Fa-f:]+)$/

/**
 * Start a new outbound: STARTTLS on port 587 with PLAIN authentication, enabled, with an empty allowlist.
 */
export function emptyOutboundForm(): OutboundForm {
  return {
    name: '', host: '', port: String(DEFAULT_PORTS.starttls), security: 'starttls', authenticationType: 'plain',
    username: '', password: '', enabled: true, overrideFrom: '', envelopeFrom: '', connectTimeout: '', smtpTimeout: '',
    tlsVerify: true, heloName: '', maxConnections: '', allowedRecipients: '',
  }
}

/**
 * Copy a stored outbound into the form; the password always starts blank.
 */
export function outboundFormFrom(outbound: SmtpOutbound): OutboundForm {
  return {
    name: outbound.name,
    host: outbound.host,
    port: String(outbound.port),
    security: outbound.security,
    authenticationType: outbound.authenticationType,
    username: outbound.username ?? '',
    password: '',
    enabled: outbound.enabled,
    overrideFrom: outbound.overrideFrom ?? '',
    envelopeFrom: outbound.envelopeFrom ?? '',
    connectTimeout: outbound.connectTimeoutSeconds === null ? '' : String(outbound.connectTimeoutSeconds),
    smtpTimeout: outbound.smtpTimeoutSeconds === null ? '' : String(outbound.smtpTimeoutSeconds),
    tlsVerify: outbound.tlsVerify,
    heloName: outbound.heloName ?? '',
    maxConnections: outbound.maxConnections === null ? '' : String(outbound.maxConnections),
    allowedRecipients: formatRecipientList(outbound.allowedRecipients),
  }
}

/**
 * Follow a security change with its conventional port, unless the user chose a custom port.
 */
export function portAfterSecurityChange(port: string, previous: OutboundSecurity, next: OutboundSecurity): string {
  return port.trim() === '' || Number(port) === DEFAULT_PORTS[previous] ? String(DEFAULT_PORTS[next]) : port
}

/**
 * Parse an optional whole number; blank is null and anything else that is not an integer is NaN.
 */
function optionalInteger(value: string): number | null {
  const text = value.trim()
  if (!text) return null
  return /^\d+$/.test(text) ? Number(text) : Number.NaN
}

/**
 * Report whether a parsed optional number is blank or inside its bounds.
 */
function withinBounds(value: number | null, [minimum, maximum]: readonly [number, number]): boolean {
  return value === null || (Number.isInteger(value) && value >= minimum && value <= maximum)
}

/**
 * Validate the form. A password is required only when authenticating without a stored one.
 */
export function outboundFormProblems(
  form: OutboundForm,
  options: { passwordConfigured: boolean; credentialsAvailable?: boolean } = { passwordConfigured: false },
): Partial<Record<OutboundFormField, OutboundFormProblem>> {
  const problems: Partial<Record<OutboundFormField, OutboundFormProblem>> = {}
  if (!form.name.trim()) problems.name = { key: 'outbound.form.error.nameRequired' }
  else if (form.name.trim().length > 100) problems.name = { key: 'outbound.form.error.nameLength' }
  if (!form.host.trim()) problems.host = { key: 'outbound.form.error.hostRequired' }
  else if (!HOST.test(form.host.trim())) problems.host = { key: 'outbound.form.error.hostInvalid' }
  const port = optionalInteger(form.port)
  if (port === null || !withinBounds(port, NUMBER_BOUNDS.port)) problems.port = { key: 'outbound.form.error.range', params: { minimum: 1, maximum: 65535 } }
  for (const field of ['connectTimeout', 'smtpTimeout', 'maxConnections'] as const) {
    const [minimum, maximum] = NUMBER_BOUNDS[field]
    if (!withinBounds(optionalInteger(form[field]), NUMBER_BOUNDS[field])) problems[field] = { key: 'outbound.form.error.range', params: { minimum, maximum } }
  }
  if (form.authenticationType !== 'none') {
    if (!form.username.trim()) problems.username = { key: 'outbound.form.error.usernameRequired' }
    if (!form.password && !options.passwordConfigured) problems.password = { key: 'outbound.form.error.passwordRequired' }
    else if (form.password && options.credentialsAvailable === false) problems.password = { key: 'outbound.form.error.credentialsUnavailable' }
  }
  for (const field of ['overrideFrom', 'envelopeFrom'] as const) {
    const address = form[field].trim()
    if (address && !isValidRecipientAddress(address)) problems[field] = { key: 'outbound.form.error.address' }
  }
  if (/\s/.test(form.heloName.trim())) problems.heloName = { key: 'outbound.form.error.hostInvalid' }
  const patterns = parseRecipientList(form.allowedRecipients)
  const invalid = patterns.filter((pattern) => !isValidRecipientPattern(pattern))
  if (invalid.length) problems.allowedRecipients = { key: 'outbound.form.error.patterns', params: { patterns: invalid.join(', ') } }
  else if (patterns.length > MAXIMUM_ALLOWED_RECIPIENTS) problems.allowedRecipients = { key: 'outbound.form.error.tooManyPatterns', params: { maximum: MAXIMUM_ALLOWED_RECIPIENTS } }
  return problems
}

/**
 * Collect the non-blocking warnings of the current settings.
 */
export function outboundFormWarnings(form: OutboundForm): OutboundFormWarnings {
  const coverage = allowlistCoverage(parseRecipientList(form.allowedRecipients))
  return {
    plaintextCredentials: form.authenticationType !== 'none' && form.security === 'none',
    certificateUnverified: form.security !== 'none' && !form.tlsVerify,
    nobodyAllowed: coverage === 'nobody',
    everyoneAllowed: coverage === 'everyone',
  }
}

/**
 * Encode a valid form for the client; call only after `outboundFormProblems` reports nothing.
 * A blank password is left out so the stored one is kept.
 */
export function outboundInputFromForm(form: OutboundForm): SmtpOutboundInput {
  const authenticates = form.authenticationType !== 'none'
  return {
    name: form.name.trim(),
    host: form.host.trim(),
    port: Number(form.port.trim()),
    security: form.security,
    authenticationType: form.authenticationType,
    username: authenticates ? form.username.trim() || null : null,
    ...(authenticates && form.password ? { password: form.password } : {}),
    enabled: form.enabled,
    overrideFrom: form.overrideFrom.trim() || null,
    envelopeFrom: form.envelopeFrom.trim() || null,
    connectTimeoutSeconds: optionalInteger(form.connectTimeout),
    smtpTimeoutSeconds: optionalInteger(form.smtpTimeout),
    tlsVerify: form.tlsVerify,
    heloName: form.heloName.trim() || null,
    maxConnections: optionalInteger(form.maxConnections),
    allowedRecipients: parseRecipientList(form.allowedRecipients),
  }
}

/**
 * Report whether any advanced setting differs from its default, so the section opens when editing.
 */
export function hasAdvancedSettings(form: OutboundForm): boolean {
  return Boolean(form.overrideFrom.trim() || form.envelopeFrom.trim() || form.connectTimeout.trim() || form.smtpTimeout.trim()
    || !form.tlsVerify || form.heloName.trim() || form.maxConnections.trim())
}
