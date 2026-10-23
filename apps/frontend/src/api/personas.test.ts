import { describe, expect, it } from 'vitest'
import type { WirePersona, WirePersonaExecution } from './wire'
import { normalizePersona, normalizePersonaExecutionDetails, normalizePersonaScriptValidation, personaWire } from './personas'

const execution: WirePersonaExecution = {
  id: 'execution-1', seq: 7, persona_id: 'persona-1', message_id: 'message-1', status: 'completed',
  scheduled_at: 100, started_at: 101, completed_at: 105, persona_revision: 2, error_message: null, subject: 'Verify',
  actions: [
    { step: 0, status: 'succeeded', line: 1, column: 1, started_at: 101, duration_milliseconds: 0, error_message: null, detail: { kind: 'branch', condition: 'subject contains "Verify"', outcome: true } },
    {
      step: 1, status: 'succeeded', line: 2, column: 5, started_at: 101, duration_milliseconds: 40, error_message: null,
      detail: { kind: 'open', images: [{ url: 'https://cdn.test/p.gif', final_url: null, status_code: 200, redirect_count: 0, content_type: 'image/gif', received_bytes: 43, truncated: false, error_kind: null, error_message: null, duration_milliseconds: 12 }], omitted_images: 0, embedded_images: 1, rejected_images: 2 },
    },
    {
      step: 2, status: 'failed', line: 3, column: 5, started_at: 102, duration_milliseconds: 9, error_message: 'refused',
      detail: { kind: 'click', selector: 'link("a")', href: '/x', text: 'Go', url: 'https://app.test/x', request: { url: 'https://app.test/x', final_url: null, status_code: null, redirect_count: 0, content_type: null, received_bytes: 0, truncated: false, error_kind: 'blocked_address', error_message: '10.0.0.1 is not a public address', duration_milliseconds: 3 } },
    },
  ],
}

describe('persona normalization', () => {
  it('converts Personas and their newest execution', () => {
    const wire: WirePersona = {
      id: 'persona-1', owner_id: 'user-1', name: 'Alice', address: 'alice@example.test', script: 'open;', enabled: false,
      revision: 3, created_at: 1, updated_at: 2, address_access: 'per_message', last_execution: execution,
    }
    expect(normalizePersona(wire)).toMatchObject({
      id: 'persona-1', ownerId: 'user-1', enabled: false, revision: 3, addressAccess: 'perMessage',
      lastExecution: { id: 'execution-1', sequence: 7, status: 'completed', subject: 'Verify', error: null },
    })
    expect(normalizePersona({ ...wire, address_access: 'confirmed', last_execution: null }).addressAccess).toBe('confirmed')
    expect(normalizePersona({ ...wire, address_access: 'denied' }).addressAccess).toBe('denied')
  })

  it('converts every step kind and request outcome', () => {
    const details = normalizePersonaExecutionDetails(execution)
    expect(details.actions.map((action) => action.detail.kind)).toEqual(['branch', 'open', 'click'])
    expect(details.actions[1]!.detail).toEqual({
      kind: 'open', omittedImages: 0, embeddedImages: 1, rejectedImages: 2,
      images: [{ url: 'https://cdn.test/p.gif', finalUrl: null, statusCode: 200, redirectCount: 0, contentType: 'image/gif', receivedBytes: 43, truncated: false, errorKind: null, error: null, durationMilliseconds: 12 }],
    })
    expect(details.actions[2]).toMatchObject({ status: 'failed', error: 'refused', detail: { kind: 'click', request: { errorKind: 'blocked_address', statusCode: null } } })
    expect(normalizePersonaExecutionDetails({ ...execution, status: 'surprising', actions: undefined })).toMatchObject({ status: 'failed', actions: [] })
  })

  it('converts validation results and request bodies', () => {
    expect(normalizePersonaScriptValidation({ valid: false, error: { message: 'unknown instruction `jump`', position: 6, line: 2, column: 1 }, statements: null, longest_wait_milliseconds: null }))
      .toEqual({ valid: false, error: { message: 'unknown instruction `jump`', position: 6, line: 2, column: 1 }, statements: null, longestWaitMilliseconds: null })
    expect(personaWire({ name: 'A', address: 'a@b.test', script: 'open;', enabled: true })).toEqual({ name: 'A', address: 'a@b.test', icon: null, script: 'open;', enabled: true })
  })

  /**
   * Preserve custom icon IDs across transport conversion and accept older responses without an icon.
   */
  it('round-trips Persona icons and explicit resets', () => {
    const definition = { name: 'Alice', address: 'alice@example.test', script: 'open;', enabled: true }
    const wire: WirePersona = {
      ...definition, id: 'persona-1', owner_id: 'user-1', revision: 1, created_at: 1, updated_at: 1,
      address_access: 'confirmed', last_execution: null,
    }
    expect(normalizePersona(wire).icon).toBeNull()
    for (const icon of ['bot', 'future-icon', null]) {
      expect(normalizePersona({ ...wire, icon }).icon).toBe(icon)
      expect(personaWire({ ...definition, icon }).icon).toBe(icon)
    }
  })
})
