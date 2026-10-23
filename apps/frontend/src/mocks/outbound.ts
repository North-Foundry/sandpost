import type {
  OutboundConnectionTest, OutboundPolicy, RoutingAction, RoutingActionInput, SandpostAdminClient, SmtpOutbound, SmtpOutboundInput,
} from '@/api/contracts'
import {
  MAXIMUM_ALLOWED_RECIPIENTS, MAXIMUM_FORWARD_RECIPIENTS, isValidRecipientAddress, recipientAllowed,
} from '@/features/outbound/recipients'

/**
 * Raise an error with the same `detail` and `status` properties the HTTP client attaches to API failures.
 */
function fail(detail: string, status = 400): never {
  throw Object.assign(new Error(detail), { detail, status })
}

/**
 * Policy of the mock instance: public destinations only, with stored credentials available.
 */
const MOCK_POLICY: OutboundPolicy = { allowInternalAddresses: false, maximumAttempts: 8, retryDelaySeconds: 60, credentialsAvailable: true }

/**
 * Reject a numeric setting outside its inclusive bounds; null keeps the server default.
 */
function checkRange(value: number | null, minimum: number, maximum: number, label: string): void {
  if (value !== null && (!Number.isInteger(value) || value < minimum || value > maximum)) fail(`${label} must be between ${minimum} and ${maximum}`)
}

/**
 * Pretend to connect: hosts ending in `.invalid` cannot be resolved, `fail` in the host name refuses the
 * connection, and authentication fails without a password. Everything else succeeds deterministically.
 */
export function mockConnectionTest(input: SmtpOutboundInput, passwordAvailable: boolean): OutboundConnectionTest {
  const host = input.host.trim().toLowerCase()
  const address = `192.0.2.25:${input.port}`
  const encrypted = input.security !== 'none'
  const result = (error: OutboundConnectionTest['error'], connected: boolean): OutboundConnectionTest => ({
    success: error === null,
    address: connected ? address : null,
    encrypted: connected && encrypted,
    authenticated: error === null && input.authenticationType !== 'none',
    extensions: connected ? ['PIPELINING', 'SIZE 10485760', 'AUTH PLAIN LOGIN', ...(input.security === 'starttls' ? ['STARTTLS'] : [])] : [],
    durationMilliseconds: connected ? 184 : 12,
    error,
  })
  if (host.endsWith('.invalid')) return result({ kind: 'connection', message: `failed to resolve ${host}`, code: null }, false)
  if (host.includes('fail')) return result({ kind: 'connection', message: `connection to ${address} refused`, code: null }, false)
  if (input.authenticationType !== 'none' && !passwordAvailable) {
    return result({ kind: 'authentication', message: '5.7.8 Authentication credentials invalid', code: 535 }, true)
  }
  return result(null, true)
}

/**
 * In-memory SMTP Outbounds, routing actions and (always empty) deliveries; the mock never sends mail.
 */
export function createMockOutbound(
  requireOwner: () => void,
  scopeExists: (scopeId: string) => boolean,
): Pick<SandpostAdminClient, 'outbound' | 'routing' | 'deliveries'> {
  /**
   * Stored outbounds; like the server, only whether a password exists is kept.
   */
  const outbounds: SmtpOutbound[] = []
  const actions: RoutingAction[] = []

  /**
   * Drop actions of deleted scopes, which the server removes together with the scope.
   */
  const pruneActions = (): void => {
    for (let index = actions.length - 1; index >= 0; index -= 1) {
      if (!scopeExists(actions[index]!.scopeId)) actions.splice(index, 1)
    }
  }

  /**
   * Require an Owner and forget actions whose scope no longer exists.
   */
  const authorize = (): void => {
    requireOwner()
    pruneActions()
  }

  /**
   * Find an outbound or fail with the server's not-found message.
   */
  const find = (id: string): SmtpOutbound => outbounds.find((outbound) => outbound.id === id) ?? fail('item not found', 404)

  /**
   * Return a detached outbound with its current routing action count.
   */
  const publicOutbound = (outbound: SmtpOutbound): SmtpOutbound => structuredClone({
    ...outbound,
    routingActionCount: actions.filter((action) => action.smtpOutboundId === outbound.id).length,
  })

  /**
   * Validate input like the server and return the stored representation, keeping a stored password when none is sent.
   */
  const build = (input: SmtpOutboundInput, existing: SmtpOutbound | null): SmtpOutbound => {
    const name = input.name.trim()
    if (!name || name.length > 100) fail('name must contain 1 to 100 printable characters')
    if (outbounds.some((outbound) => outbound.id !== existing?.id && outbound.name.toLowerCase() === name.toLowerCase())) {
      fail(`an SMTP outbound named "${name}" already exists`, 409)
    }
    if (!input.host.trim()) fail('host must not be empty')
    checkRange(input.port, 1, 65535, 'port')
    checkRange(input.connectTimeoutSeconds, 1, 300, 'connect timeout')
    checkRange(input.smtpTimeoutSeconds, 1, 600, 'SMTP timeout')
    checkRange(input.maxConnections, 1, 16, 'max connections')
    if (input.allowedRecipients.length > MAXIMUM_ALLOWED_RECIPIENTS) fail(`at most ${MAXIMUM_ALLOWED_RECIPIENTS} allowed recipient patterns are accepted`)
    const authenticates = input.authenticationType !== 'none'
    const passwordConfigured = authenticates && Boolean(input.password || existing?.passwordConfigured)
    if (authenticates && !input.username?.trim()) fail('authentication requires a username')
    if (authenticates && !passwordConfigured) fail('authentication requires a password')
    const now = Date.now()
    return {
      id: existing?.id ?? crypto.randomUUID(),
      name,
      host: input.host.trim(),
      port: input.port,
      security: input.security,
      authenticationType: input.authenticationType,
      username: authenticates ? input.username?.trim() ?? null : null,
      passwordConfigured,
      enabled: input.enabled ?? existing?.enabled ?? false,
      overrideFrom: input.overrideFrom,
      envelopeFrom: input.envelopeFrom,
      connectTimeoutSeconds: input.connectTimeoutSeconds,
      smtpTimeoutSeconds: input.smtpTimeoutSeconds,
      tlsVerify: input.tlsVerify,
      heloName: input.heloName,
      maxConnections: input.maxConnections,
      allowedRecipients: [...input.allowedRecipients],
      routingActionCount: 0,
      createdAt: existing?.createdAt ?? now,
      updatedAt: now,
    }
  }

  /**
   * Validate a routing action against its outbound's allowlist and return the stored fields.
   */
  const buildAction = (input: RoutingActionInput): Pick<RoutingAction, 'smtpOutboundId' | 'actionType' | 'forwardRecipients' | 'enabled'> => {
    const outbound = outbounds.find((candidate) => candidate.id === input.smtpOutboundId) ?? fail('the SMTP outbound does not exist')
    const forwardRecipients = input.actionType === 'forward' ? input.forwardRecipients.map((address) => address.trim()).filter(Boolean) : []
    if (input.actionType === 'forward') {
      if (!forwardRecipients.length || forwardRecipients.length > MAXIMUM_FORWARD_RECIPIENTS) {
        fail(`a forward action needs 1 to ${MAXIMUM_FORWARD_RECIPIENTS} recipients`)
      }
      const invalid = forwardRecipients.find((address) => !isValidRecipientAddress(address))
      if (invalid) fail(`${invalid} is not a valid email address`)
      const refused = forwardRecipients.find((address) => !recipientAllowed(address, outbound.allowedRecipients))
      if (refused) fail(`${refused} is not allowed by the recipients allowlist of ${outbound.name}`)
    }
    return { smtpOutboundId: outbound.id, actionType: input.actionType, forwardRecipients, enabled: input.enabled ?? true }
  }

  return {
    outbound: {
      /**
       * Return the mock instance policy.
       */
      async policy() { authorize(); return { ...MOCK_POLICY } },

      /**
       * List outbounds ordered by name.
       */
      async list() {
        authorize()
        return outbounds.map(publicOutbound).sort((first, second) => first.name.localeCompare(second.name))
      },

      /**
       * Read one outbound.
       */
      async get(id) { authorize(); return publicOutbound(find(id)) },

      /**
       * Create an outbound, disabled unless requested otherwise.
       */
      async create(input) {
        authorize()
        const outbound = build(input, null)
        outbounds.push(outbound)
        return publicOutbound(outbound)
      },

      /**
       * Replace an outbound, keeping the stored password when none is sent.
       */
      async update(id, input) {
        authorize()
        const index = outbounds.indexOf(find(id))
        outbounds[index] = build(input, outbounds[index]!)
        return publicOutbound(outbounds[index]!)
      },

      /**
       * Enable or disable an outbound.
       */
      async setEnabled(id, enabled) {
        authorize()
        const outbound = find(id)
        outbound.enabled = enabled
        outbound.updatedAt = Date.now()
        return publicOutbound(outbound)
      },

      /**
       * Delete an outbound that no routing action references.
       */
      async delete(id) {
        authorize()
        const outbound = find(id)
        const references = actions.filter((action) => action.smtpOutboundId === id).length
        if (references) fail(`${outbound.name} is used by ${references} routing action${references === 1 ? '' : 's'}; delete them first`, 409)
        outbounds.splice(outbounds.indexOf(outbound), 1)
      },

      /**
       * Test the stored settings with the deterministic fake connection.
       */
      async test(id) {
        authorize()
        const outbound = find(id)
        return mockConnectionTest(outbound, outbound.passwordConfigured)
      },

      /**
       * Test unsaved settings, reusing the stored password of `id` when none is typed.
       */
      async testDraft(input, id) {
        authorize()
        const stored = id ? find(id) : null
        return mockConnectionTest(input, Boolean(input.password) || Boolean(stored?.passwordConfigured))
      },
    },
    routing: {
      /**
       * List a scope's actions in creation order.
       */
      async list(scopeId) {
        authorize()
        if (!scopeExists(scopeId)) fail('item not found', 404)
        return structuredClone(actions.filter((action) => action.scopeId === scopeId))
      },

      /**
       * Add an action to a saved scope.
       */
      async create(scopeId, input) {
        authorize()
        if (!scopeExists(scopeId)) fail('item not found', 404)
        const now = Date.now()
        const action: RoutingAction = { id: crypto.randomUUID(), scopeId, ...buildAction(input), createdAt: now, updatedAt: now }
        actions.push(action)
        return structuredClone(action)
      },

      /**
       * Replace an action's outbound, type, recipients and state.
       */
      async update(id, input) {
        authorize()
        const action = actions.find((candidate) => candidate.id === id) ?? fail('item not found', 404)
        Object.assign(action, buildAction(input), { updatedAt: Date.now() })
        return structuredClone(action)
      },

      /**
       * Delete an action.
       */
      async delete(id) {
        authorize()
        const index = actions.findIndex((action) => action.id === id)
        if (index < 0) fail('item not found', 404)
        actions.splice(index, 1)
      },
    },
    deliveries: {
      /**
       * The mock never delivers mail, so there are no deliveries.
       */
      async list() { authorize(); return [] },

      /**
       * No delivery exists in the mock.
       */
      async get() { authorize(); return fail('item not found', 404) },

      /**
       * No delivery exists in the mock.
       */
      async retry() { authorize(); return fail('item not found', 404) },

      /**
       * Mock messages are never delivered onwards.
       */
      async forMessage() { authorize(); return [] },
    },
  }
}
