import type {
  LinkCheckErrorKind,
  Persona,
  PersonaActionDetail,
  PersonaActionStatus,
  PersonaAddressAccess,
  PersonaExecutionAction,
  PersonaExecutionDetails,
  PersonaExecutionStatus,
  PersonaExecutionSummary,
  PersonaHttpRequest,
  PersonaInput,
  PersonaScriptValidation,
} from './contracts'
import type {
  WirePersona,
  WirePersonaActionDetail,
  WirePersonaExecution,
  WirePersonaExecutionAction,
  WirePersonaHttpRequest,
  WirePersonaScriptValidation,
} from './wire'

const EXECUTION_STATUSES: readonly PersonaExecutionStatus[] = ['pending', 'running', 'completed', 'failed', 'cancelled']
const ACTION_STATUSES: readonly PersonaActionStatus[] = ['started', 'succeeded', 'failed', 'interrupted', 'cancelled']

/**
 * Convert the Rust address access; unknown values are treated as decided per message.
 */
function normalizeAddressAccess(value: string): PersonaAddressAccess {
  if (value === 'confirmed' || value === 'denied') return value
  return 'perMessage'
}

/**
 * Convert an execution status, treating unknown values as failed so they never look successful.
 */
function normalizeExecutionStatus(value: string): PersonaExecutionStatus {
  return EXECUTION_STATUSES.find((status) => status === value) ?? 'failed'
}

/**
 * Convert a step status, treating unknown values as failed.
 */
function normalizeActionStatus(value: string): PersonaActionStatus {
  return ACTION_STATUSES.find((status) => status === value) ?? 'failed'
}

/**
 * Convert one outbound request outcome.
 */
function normalizeHttpRequest(representation: WirePersonaHttpRequest): PersonaHttpRequest {
  return {
    url: String(representation.url),
    finalUrl: representation.final_url ?? null,
    statusCode: representation.status_code == null ? null : Number(representation.status_code),
    redirectCount: Number(representation.redirect_count ?? 0),
    contentType: representation.content_type ?? null,
    receivedBytes: Number(representation.received_bytes ?? 0),
    truncated: Boolean(representation.truncated),
    errorKind: (representation.error_kind as LinkCheckErrorKind | null) ?? null,
    error: representation.error_message ?? null,
    durationMilliseconds: Number(representation.duration_milliseconds ?? 0),
  }
}

/**
 * Convert one step detail by kind.
 */
function normalizeDetail(detail: WirePersonaActionDetail): PersonaActionDetail {
  switch (detail.kind) {
    case 'branch':
      return { kind: 'branch', condition: String(detail.condition), outcome: Boolean(detail.outcome) }
    case 'open':
      return {
        kind: 'open',
        images: (detail.images ?? []).map(normalizeHttpRequest),
        omittedImages: Number(detail.omitted_images ?? 0),
        embeddedImages: Number(detail.embedded_images ?? 0),
        rejectedImages: Number(detail.rejected_images ?? 0),
      }
    case 'wait':
      return { kind: 'wait', milliseconds: Number(detail.milliseconds) }
    default:
      return {
        kind: 'click',
        selector: String(detail.selector),
        href: detail.href ?? null,
        text: detail.text ?? null,
        url: detail.url ?? null,
        request: detail.request ? normalizeHttpRequest(detail.request) : null,
      }
  }
}

/**
 * Convert one recorded step.
 */
function normalizeAction(representation: WirePersonaExecutionAction): PersonaExecutionAction {
  return {
    step: Number(representation.step),
    status: normalizeActionStatus(representation.status),
    line: Number(representation.line),
    column: Number(representation.column),
    startedAt: Number(representation.started_at),
    durationMilliseconds: representation.duration_milliseconds == null ? null : Number(representation.duration_milliseconds),
    error: representation.error_message ?? null,
    detail: normalizeDetail(representation.detail),
  }
}

/**
 * Convert an execution summary.
 */
export function normalizePersonaExecution(representation: WirePersonaExecution): PersonaExecutionSummary {
  return {
    id: String(representation.id),
    sequence: Number(representation.seq),
    personaId: String(representation.persona_id),
    messageId: String(representation.message_id),
    status: normalizeExecutionStatus(representation.status),
    scheduledAt: Number(representation.scheduled_at),
    startedAt: representation.started_at == null ? null : Number(representation.started_at),
    completedAt: representation.completed_at == null ? null : Number(representation.completed_at),
    error: representation.error_message ?? null,
    subject: String(representation.subject ?? ''),
  }
}

/**
 * Convert an execution with its recorded steps.
 */
export function normalizePersonaExecutionDetails(representation: WirePersonaExecution): PersonaExecutionDetails {
  return { ...normalizePersonaExecution(representation), actions: (representation.actions ?? []).map(normalizeAction) }
}

/**
 * Convert Persona details.
 */
export function normalizePersona(representation: WirePersona): Persona {
  return {
    id: String(representation.id),
    ownerId: String(representation.owner_id),
    name: String(representation.name),
    address: String(representation.address),
    script: String(representation.script),
    enabled: Boolean(representation.enabled),
    revision: Number(representation.revision),
    createdAt: Number(representation.created_at),
    updatedAt: Number(representation.updated_at),
    addressAccess: normalizeAddressAccess(representation.address_access),
    lastExecution: representation.last_execution ? normalizePersonaExecution(representation.last_execution) : null,
  }
}

/**
 * Convert a PersonaScript validation result.
 */
export function normalizePersonaScriptValidation(representation: WirePersonaScriptValidation): PersonaScriptValidation {
  return {
    valid: Boolean(representation.valid),
    error: representation.error
      ? {
        message: String(representation.error.message),
        position: Number(representation.error.position),
        line: Number(representation.error.line),
        column: Number(representation.error.column),
      }
      : null,
    statements: representation.statements == null ? null : Number(representation.statements),
    longestWaitMilliseconds: representation.longest_wait_milliseconds == null ? null : Number(representation.longest_wait_milliseconds),
  }
}

/**
 * Build the Rust Persona request body.
 */
export function personaWire(input: PersonaInput): { name: string; address: string; script: string; enabled: boolean } {
  return { name: input.name, address: input.address, script: input.script, enabled: input.enabled }
}
