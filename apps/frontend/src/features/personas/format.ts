/**
 * Presentation helpers for Persona execution history.
 */
import type { PersonaActionStatus, PersonaExecutionStatus, PersonaHttpRequest } from '@/api/contracts'

export type StatusTone = 'success' | 'destructive' | 'warning' | 'muted' | 'progress'

/**
 * Color an execution status: completed is green, failed red, cancelled muted, and unfinished in progress.
 */
export function executionTone(status: PersonaExecutionStatus): StatusTone {
  switch (status) {
    case 'completed': return 'success'
    case 'failed': return 'destructive'
    case 'cancelled': return 'muted'
    default: return 'progress'
  }
}

/**
 * Color a step status the same way; interrupted steps are warnings because their request may have happened.
 */
export function actionTone(status: PersonaActionStatus): StatusTone {
  switch (status) {
    case 'succeeded': return 'success'
    case 'failed': return 'destructive'
    case 'interrupted': return 'warning'
    case 'cancelled': return 'muted'
    default: return 'progress'
  }
}

/**
 * Utility classes for a status dot of each tone.
 */
export const TONE_DOT: Record<StatusTone, string> = {
  success: 'bg-success',
  destructive: 'bg-destructive',
  warning: 'bg-warning',
  muted: 'bg-muted-foreground/50',
  progress: 'bg-primary animate-pulse',
}

/**
 * Report whether an execution can still change, so its view should refresh.
 */
export function isUnfinished(status: PersonaExecutionStatus): boolean {
  return status === 'pending' || status === 'running'
}

/**
 * Format a duration in the largest whole unit that keeps it readable, using the locale's unit names.
 */
export function formatDuration(milliseconds: number, locale: string): string {
  const format = (value: number, unit: 'millisecond' | 'second' | 'minute') =>
    new Intl.NumberFormat(locale, { style: 'unit', unit, unitDisplay: 'short', maximumFractionDigits: 1 }).format(value)
  if (milliseconds < 1000) return format(milliseconds, 'millisecond')
  if (milliseconds < 60_000) return format(milliseconds / 1000, 'second')
  return format(milliseconds / 60_000, 'minute')
}

/**
 * Report whether a request obtained a final response, whatever its status code.
 */
export function responded(request: PersonaHttpRequest): boolean {
  return request.statusCode !== null && request.errorKind === null
}

/**
 * Color a request: 2xx and 3xx green, 4xx and 5xx red, no response yellow.
 */
export function requestTone(request: PersonaHttpRequest): StatusTone {
  if (!responded(request)) return 'warning'
  return request.statusCode! < 400 ? 'success' : 'destructive'
}
