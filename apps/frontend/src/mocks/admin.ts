import type {
  IssuedSmtpCredentials, ListenerStatus, MessageSummary, QueryValidation, SandpostAdminClient, Scope, SessionUser, SmtpAccess, SmtpMechanism,
  SmtpServer, UserAccount,
} from '@/api/contracts'
import { SCOPE_PREVIEW_LIMIT } from '@/api/admin'

/**
 * Ports the mock pretends another process already holds, to exercise failed listeners.
 */
const OCCUPIED_PORTS = new Set([25, 8025])
const PASSWORD_ALPHABET = 'ABCDEFGHJKLMNPQRSTUVWXYZabcdefghijkmnpqrstuvwxyz23456789'
const USERNAME = /^[A-Za-z0-9][A-Za-z0-9._+@-]{2,63}$/
const OPERATORS = /==|!=|>=|<=|>|<|\bcontains\b|\bstarts_with\b|\bends_with\b|\bmatches\b/i

/**
 * Raise an error with the same `detail` property the HTTP client attaches to API failures.
 */
function fail(detail: string): never {
  throw Object.assign(new Error(detail), { detail })
}

/**
 * Simulate the runtime binding the global server's saved listener address.
 */
function mockListener(address: string | null): ListenerStatus {
  if (!address) return { configuredAddress: null, boundAddress: null, source: 'configuration', state: 'disabled', error: null, authenticationRequired: false, encryption: 'none' }
  const separator = address.lastIndexOf(':')
  const port = Number(address.slice(separator + 1))
  const host = address.slice(0, separator).replace(/^\[|\]$/g, '')
  const authenticationRequired = host !== '127.0.0.1' && host !== '::1'
  return OCCUPIED_PORTS.has(port)
    ? { configuredAddress: address, boundAddress: null, source: 'configuration', state: 'failed', error: `${address} is already in use by another process`, authenticationRequired, encryption: 'none' }
    : { configuredAddress: address, boundAddress: address, source: 'configuration', state: 'listening', error: null, authenticationRequired, encryption: 'none' }
}

/**
 * Generate a mock SMTP or IMAP password with the same shape as the server's.
 */
export function mockPassword(): string {
  const bytes = crypto.getRandomValues(new Uint8Array(32))
  return Array.from(bytes, (byte) => PASSWORD_ALPHABET[byte % PASSWORD_ALPHABET.length]).join('')
}

/**
 * Approximate query validation: balanced quotes and parentheses plus at least one comparison.
 */
export function validateMockQuery(query: string): QueryValidation {
  const source = query.trim()
  if (!source || /^(true|false)$/i.test(source)) return { valid: true, error: null }
  let depth = 0
  let quote = -1
  for (let index = 0; index < query.length; index += 1) {
    const character = query[index]
    if (quote >= 0) {
      if (character === '\\') index += 1
      else if (character === '"') quote = -1
      continue
    }
    if (character === '"') quote = index
    else if (character === '(') depth += 1
    else if (character === ')' && --depth < 0) return { valid: false, error: { message: 'unexpected closing parenthesis', position: new TextEncoder().encode(query.slice(0, index)).length } }
  }
  const end = new TextEncoder().encode(query).length
  if (quote >= 0) return { valid: false, error: { message: 'unterminated string', position: new TextEncoder().encode(query.slice(0, quote)).length } }
  if (depth > 0) return { valid: false, error: { message: 'expected closing parenthesis', position: end } }
  if (!OPERATORS.test(query)) return { valid: false, error: { message: 'expected a comparison operator', position: end } }
  return { valid: true, error: null }
}

/**
 * Mock matching: every quoted literal in every filter must appear in the message's text.
 */
export function matches(message: MessageSummary, filters: string[]): boolean {
  const haystack = [message.subject, message.preview ?? '', ...message.from, ...message.to].join(' ').toLowerCase()
  return filters.every((filter) => [...filter.matchAll(/"((?:[^"\\]|\\.)*)"/g)]
    .every(([, literal]) => haystack.includes(literal!.toLowerCase())))
}

/**
 * In-memory administration state for UI development without a server.
 */
export function createMockAdmin(activeUser: () => SessionUser | null, summaries: MessageSummary[]): SandpostAdminClient {
  const scopes: Scope[] = []
  const assignments = new Map<string, Set<string>>()

  /**
   * Mock SMTP accesses; like the server, only a fake hash is kept, never the password.
   */
  const accesses: (SmtpAccess & { passwordHash: string })[] = []
  const serverAddress: string | null = '127.0.0.1:1025'
  const users: UserAccount[] = [
    { id: '20000000-0000-4000-8000-000000000001', name: 'Elena Rossi', email: 'elena@northfoundry.dev', role: 'admin', mailAccess: 'all' },
    { id: '20000000-0000-4000-8000-000000000002', name: 'Luca Bianchi', email: 'luca@northfoundry.dev', role: 'member', mailAccess: 'scoped' },
    { id: '20000000-0000-4000-8000-000000000003', name: 'Marta Greco', email: 'marta@northfoundry.dev', role: 'viewer', mailAccess: 'all' },
  ]

  /**
   * Return the configured listener address with its simulated bind result.
   */
  const server = (): SmtpServer => ({ listenAddress: serverAddress, listener: mockListener(serverAddress) })

  /**
   * Keep supported mechanisms in canonical order and reject an empty set.
   */
  const validMechanisms = (mechanisms: SmtpMechanism[]): SmtpMechanism[] => {
    const allowed = (['plain', 'login'] as const).filter((mechanism) => mechanisms.includes(mechanism))
    return allowed.length ? allowed : fail('allow at least one SMTP AUTH mechanism')
  }

  /**
   * Remove credential hashes from public SMTP access values.
   */
  const publicAccess = ({ passwordHash: _hash, ...access }: SmtpAccess & { passwordHash: string }): SmtpAccess => ({ ...access })

  /**
   * Issue a one-time password while retaining only a fake hash in mock state.
   */
  const issue = (access: SmtpAccess & { passwordHash: string }): IssuedSmtpCredentials => {
    const password = mockPassword()
    access.passwordHash = `mock-hash-${password.length}-${crypto.randomUUID()}`
    return { access: publicAccess(access), password, server: server() }
  }

  /**
   * Require an authenticated owner or administrator for SMTP management operations.
   */
  const requireGlobalAdmin = () => {
    const user = requireUser()
    if (user.role !== 'owner' && user.role !== 'admin') fail('forbidden')
  }

  /**
   * Return the active user or fail with the mock's authentication error shape.
   */
  const requireUser = () => activeUser() ?? fail('authentication required')

  /**
   * Include an externally supplied owner in the mock directory when necessary.
   */
  const userList = () => {
    const owner = activeUser()
    return owner && !users.some((user) => user.id === owner.id) ? [{ ...owner, mailAccess: 'all' as const }, ...users] : users
  }

  /**
   * Collect filters from a scope and its ancestors, failing for a missing ancestor.
   */
  const ancestry = (parentId: string | null): string[] => {
    const filters: string[] = []
    for (let current = parentId; current;) {
      const scope = scopes.find((candidate) => candidate.id === current) ?? fail('item not found')
      filters.push(scope.filter)
      current = scope.parentId
    }
    return filters
  }

  return {
    smtp: {

      /**
       * Return listener details after enforcing global-administrator access.
       */
      async server() { requireGlobalAdmin(); return server() },

      /**
       * List SMTP accesses without exposing their stored mock hashes.
       */
      async list() { requireGlobalAdmin(); return accesses.map(publicAccess) },

      /**
       * Validate and create an SMTP access, then return its generated password once.
       */
      async create(input) {
        requireGlobalAdmin()
        const name = input.name.trim()
        if (!name || name.length > 100) fail('name must contain 1 to 100 printable characters')
        const requested = input.username?.trim()
        if (requested && !USERNAME.test(requested)) fail('username must contain 3 to 64 letters, digits, or . _ - + @ and start with a letter or digit')
        if (requested && accesses.some((access) => access.username === requested)) fail('conflict')
        const slug = name.toLowerCase().replace(/[^a-z0-9]+/g, '-').replace(/^-|-$/g, '').slice(0, 32) || 'smtp'
        const access = {
          id: crypto.randomUUID(), name, username: requested || `${slug}-${crypto.randomUUID().slice(0, 6)}`,
          enabled: true, createdAt: Date.now(), lastUsedAt: null, passwordHash: '',
          requiresEncryption: input.requiresEncryption ?? false,
          allowedMechanisms: validMechanisms(input.allowedMechanisms ?? ['plain', 'login']),
        }
        accesses.push(access)
        return issue(access)
      },

      /**
       * Replace an access password and return the new one-time credential.
       */
      async regeneratePassword(id) {
        requireGlobalAdmin()
        return issue(accesses.find((access) => access.id === id) ?? fail('item not found'))
      },

      /**
       * Change whether an SMTP access is enabled.
       */
      async setEnabled(id, enabled) {
        requireGlobalAdmin()
        const access = accesses.find((candidate) => candidate.id === id) ?? fail('item not found')
        access.enabled = enabled
        return publicAccess(access)
      },

      /**
       * Replace encryption and AUTH mechanism rules after validating the mechanism set.
       */
      async updateRules(id, rules) {
        requireGlobalAdmin()
        const access = accesses.find((candidate) => candidate.id === id) ?? fail('item not found')
        access.allowedMechanisms = validMechanisms(rules.allowedMechanisms)
        access.requiresEncryption = rules.requiresEncryption
        return publicAccess(access)
      },

      /**
       * Delete an SMTP access or fail when the identifier is unknown.
       */
      async delete(id) {
        requireGlobalAdmin()
        const index = accesses.findIndex((access) => access.id === id)
        if (index < 0) fail('item not found')
        accesses.splice(index, 1)
      },
    },
    scopes: {

      /**
       * Return a detached snapshot of all mock scopes to any authenticated user.
       */
      async list() { requireUser(); return structuredClone(scopes) },

      /**
       * Validate and insert or replace a scope, incrementing its policy version on update.
       */
      async save(input) {
        requireUser()
        const name = input.name.trim()
        if (!name) fail('name must contain 1 to 100 printable characters')
        const validation = validateMockQuery(input.filter)
        if (!validation.valid) fail(validation.error!.message)
        for (let current = input.parentId; current;) {
          if (current === input.id) fail('a scope cannot be its own ancestor')
          current = scopes.find((scope) => scope.id === current)?.parentId ?? null
        }
        const index = scopes.findIndex((scope) => scope.id === input.id)
        const saved: Scope = { ...input, name, policyVersion: index >= 0 ? scopes[index]!.policyVersion + 1 : 1 }
        if (index >= 0) scopes[index] = saved
        else scopes.push(saved)
        return structuredClone(saved)
      },

      /**
       * Delete a childless scope and discard assignments attached to its identifier.
       */
      async delete(id) {
        requireUser()
        if (scopes.some((scope) => scope.parentId === id)) fail('delete or move its child scopes first')
        const index = scopes.findIndex((scope) => scope.id === id)
        if (index < 0) fail('item not found')
        scopes.splice(index, 1)
        assignments.delete(id)
      },

      /**
       * Preview messages matching this filter and every ancestor filter using approximate matching.
       */
      async preview({ parentId, filter, limit }) {
        requireUser()
        const validation = validateMockQuery(filter)
        if (!validation.valid) fail(validation.error!.message)
        const filters = [filter, ...ancestry(parentId)]
        return summaries.filter((message) => matches(message, filters)).slice(0, limit ?? SCOPE_PREVIEW_LIMIT).map((message) => ({ ...message }))
      },

      /**
       * Return the assigned user identifiers for a scope, or an empty list.
       */
      async assignments(scopeId) { requireUser(); return [...(assignments.get(scopeId) ?? [])] },

      /**
       * Add a validated user to a scope's in-memory assignment set.
       */
      async assign(scopeId, userId) {
        requireGlobalAdmin()
        scopes.find((candidate) => candidate.id === scopeId) ?? fail('item not found')
        userList().find((candidate) => candidate.id === userId) ?? fail('item not found')
        assignments.set(scopeId, new Set([...(assignments.get(scopeId) ?? []), userId]))
      },

      /**
       * Remove an assignment if present; missing scopes and users are ignored by this mock.
       */
      async unassign(scopeId, userId) { requireUser(); assignments.get(scopeId)?.delete(userId) },
    },
    users: {

      /**
       * Return a detached snapshot of the mock user directory.
       */
      async list() { requireUser(); return structuredClone(userList()) },

      /**
       * Add a mock user after checking email uniqueness and minimum UTF-8 password length.
       */
      async create(input) {
        requireUser()
        const email = input.email.trim().toLowerCase()
        if (userList().some((user) => user.email === email)) fail('conflict')
        if (new TextEncoder().encode(input.password).length < 12) fail('password must contain 12 to 1024 bytes')
        const user: UserAccount = { id: crypto.randomUUID(), name: input.name.trim(), email, role: input.role, mailAccess: input.mailAccess }
        users.push(user)
        return { ...user }
      },

      /**
       * Apply only supplied user fields; changed values are not revalidated by this mock.
       */
      async update(id, changes) {
        requireUser()
        const user = users.find((candidate) => candidate.id === id) ?? fail('the mock owner cannot be edited')
        if (changes.name !== undefined) user.name = changes.name.trim()
        if (changes.role !== undefined) user.role = changes.role
        if (changes.mailAccess !== undefined) user.mailAccess = changes.mailAccess
        return { ...user }
      },

      /**
       * Delete a user and remove that identifier from every scope assignment.
       */
      async delete(id) {
        requireUser()
        const index = users.findIndex((user) => user.id === id)
        if (index < 0) fail('the mock owner cannot be deleted')
        users.splice(index, 1)
        for (const assigned of assignments.values()) assigned.delete(id)
      },
    },

    /**
     * Validate a query with the mock's limited syntax checks after authentication.
     */
    async validateQuery(query) { requireUser(); return validateMockQuery(query) },
  }
}
