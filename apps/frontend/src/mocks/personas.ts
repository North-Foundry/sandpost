import type { MessageSummary, Persona, PersonaInput, PersonaScriptValidation, SandpostPersonaClient, SessionUser } from '@/api/contracts'
import { normalizeInboxAddress } from '@/api/inboxAddress'
import { matches } from './admin'

/**
 * The application's per-user Persona bound.
 */
const MAXIMUM_PERSONAS_PER_USER = 50
const INSTRUCTIONS = new Set(['open', 'wait', 'click', 'if', 'else'])

/**
 * Raise an error with the same `detail` property the HTTP client attaches to API failures.
 */
function fail(detail: string): never {
  throw Object.assign(new Error(detail), { detail })
}

/**
 * Approximate PersonaScript validation: every statement must start with a known instruction and blocks must
 * balance. The server parses the full grammar and compiles conditions with the query language.
 */
export function validateMockScript(script: string): PersonaScriptValidation {
  let depth = 0
  let line = 1
  let column = 1
  let expectStatement = true
  for (let index = 0; index < script.length; index += 1) {
    const character = script[index]!
    if (character === '\n') { line += 1; column = 1; continue }
    if (expectStatement && /[A-Za-z_]/.test(character)) {
      const word = /^[A-Za-z_][A-Za-z0-9_]*/.exec(script.slice(index))![0]
      if (!INSTRUCTIONS.has(word)) {
        return {
          valid: false,
          error: { message: `unknown instruction \`${word}\`; expected \`open\`, \`wait\`, \`click\` or \`if\``, position: new TextEncoder().encode(script.slice(0, index)).length, line, column },
          statements: null,
          longestWaitMilliseconds: null,
        }
      }
      expectStatement = false
    }
    if (character === '{') { depth += 1; expectStatement = true }
    if (character === '}') { depth -= 1; expectStatement = true }
    if (character === ';') expectStatement = true
    if (depth < 0) break
    column += 1
  }
  if (depth !== 0) {
    return { valid: false, error: { message: 'unbalanced block braces', position: new TextEncoder().encode(script).length, line, column }, statements: null, longestWaitMilliseconds: null }
  }
  const waits = [...script.matchAll(/\bwait\s+(\d+)(ms|s|m)\b/g)]
    .reduce((total, [, amount, unit]) => total + Number(amount) * (unit === 'ms' ? 1 : unit === 's' ? 1000 : 60_000), 0)
  return { valid: true, error: null, statements: (script.match(/;/g) ?? []).length, longestWaitMilliseconds: waits }
}

/**
 * Mock Persona management over the fixed message fixtures. Scripts are stored but never executed, so execution
 * history stays empty; the Rust application runs them.
 */
export function createMockPersonas(
  currentUser: () => SessionUser | null,
  summaries: () => MessageSummary[],
): SandpostPersonaClient {
  const personas = new Map<string, Persona>()

  /**
   * Require a signed-in user.
   */
  const requireUser = (): SessionUser => currentUser() ?? fail('authentication_required')

  /**
   * Load a Persona the caller owns, or any Persona for an Owner.
   */
  const visible = (id: string): Persona => {
    const user = requireUser()
    const persona = personas.get(id)
    if (!persona || (persona.ownerId !== user.id && user.role !== 'owner')) fail('item not found')
    return persona
  }

  /**
   * Validate and normalize a definition the way the server does.
   */
  const validated = (input: PersonaInput, excludedId: string | null): PersonaInput => {
    const name = input.name.trim()
    if (!name) fail('name must not be empty')
    let address: string
    try {
      address = normalizeInboxAddress(input.address)
    } catch {
      fail('address must be a valid ASCII email address with a 1 to 64 character dot-atom local part')
    }
    const validation = validateMockScript(input.script)
    if (!validation.valid) fail(`invalid PersonaScript at line ${validation.error!.line}, column ${validation.error!.column}: ${validation.error!.message}`)
    if ([...personas.values()].some((persona) => persona.address === address && persona.id !== excludedId)) {
      fail('this email address is already used by a persona; choose another address')
    }
    return { name, address, script: input.script, enabled: input.enabled }
  }

  /**
   * Return a detached copy so callers cannot mutate mock state.
   */
  const copy = (persona: Persona): Persona => structuredClone(persona)

  return {
    /**
     * List the caller's own Personas by name.
     */
    async list() {
      const user = requireUser()
      return [...personas.values()].filter((persona) => persona.ownerId === user.id)
        .sort((first, second) => first.name.localeCompare(second.name)).map(copy)
    },

    /**
     * Read one visible Persona.
     */
    async get(id) { return copy(visible(id)) },

    /**
     * Create a Persona owned by the caller.
     */
    async create(input) {
      const user = requireUser()
      if (user.role === 'viewer') fail('forbidden')
      if ([...personas.values()].filter((persona) => persona.ownerId === user.id).length >= MAXIMUM_PERSONAS_PER_USER) {
        fail(`a user can have at most ${MAXIMUM_PERSONAS_PER_USER} personas`)
      }
      const definition = validated(input, null)
      const now = Math.floor(Date.now() / 1000)
      const persona: Persona = {
        ...definition, id: crypto.randomUUID(), ownerId: user.id, revision: 1, createdAt: now, updatedAt: now,
        addressAccess: 'confirmed', lastExecution: null,
      }
      personas.set(persona.id, persona)
      return copy(persona)
    },

    /**
     * Replace the caller's own Persona definition.
     */
    async update(id, input) {
      const persona = visible(id)
      if (persona.ownerId !== requireUser().id) fail('forbidden')
      const updated = { ...persona, ...validated(input, id), revision: persona.revision + 1, updatedAt: Math.floor(Date.now() / 1000) }
      personas.set(id, updated)
      return copy(updated)
    },

    /**
     * Enable or disable a visible Persona.
     */
    async setEnabled(id, enabled) {
      const persona = visible(id)
      if (requireUser().role === 'viewer') fail('forbidden')
      const updated = { ...persona, enabled, revision: persona.revision + 1, updatedAt: Math.floor(Date.now() / 1000) }
      personas.set(id, updated)
      return copy(updated)
    },

    /**
     * Delete a visible Persona.
     */
    async delete(id) {
      visible(id)
      if (requireUser().role === 'viewer') fail('forbidden')
      personas.delete(id)
    },

    /**
     * Approximate validation; the server is authoritative.
     */
    async validate(script) { return validateMockScript(script) },

    /**
     * List fixture mail whose recipients include the Persona's address.
     */
    async listMessages(id, options = {}) {
      const persona = visible(id)
      const limit = options.limit ?? 50
      return summaries()
        .filter((message) => [...(message.envelopeTo ?? message.to)].some((address) => address.toLowerCase() === persona.address))
        .filter((message) => options.before === undefined || message.sequence < options.before)
        .filter((message) => !options.query || matches(message, [options.query]))
        .slice(0, limit)
    },

    /**
     * The mock never executes scripts.
     */
    async executions(id) { visible(id); return [] },

    /**
     * The mock never executes scripts.
     */
    async execution(id) { visible(id); return fail('item not found') },
  }
}
