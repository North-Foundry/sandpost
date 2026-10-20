import { describe, expect, it } from 'vitest'
import type { PersonaHttpRequest } from '@/api/contracts'
import { actionTone, executionTone, formatDuration, isUnfinished, requestTone, responded } from './format'

/**
 * Build a request outcome with overrides.
 */
function request(overrides: Partial<PersonaHttpRequest>): PersonaHttpRequest {
  return {
    url: 'https://example.test/', finalUrl: null, statusCode: 200, redirectCount: 0, contentType: null,
    receivedBytes: 0, truncated: false, errorKind: null, error: null, durationMilliseconds: 10, ...overrides,
  }
}

describe('persona history formatting', () => {
  it('colors execution and step statuses by outcome', () => {
    expect(executionTone('completed')).toBe('success')
    expect(executionTone('failed')).toBe('destructive')
    expect(executionTone('cancelled')).toBe('muted')
    expect(executionTone('running')).toBe('progress')
    expect(actionTone('interrupted')).toBe('warning')
    expect(isUnfinished('pending')).toBe(true)
    expect(isUnfinished('completed')).toBe(false)
  })

  it('formats waits with localized units', () => {
    expect(formatDuration(500, 'en')).toBe('500 ms')
    expect(formatDuration(5000, 'en')).toBe('5 sec')
    expect(formatDuration(120_000, 'en')).toBe('2 min')
    expect(formatDuration(1500, 'de')).toContain('1,5')
  })

  it('distinguishes responses from transport failures', () => {
    expect(responded(request({}))).toBe(true)
    expect(requestTone(request({ statusCode: 302 }))).toBe('success')
    expect(requestTone(request({ statusCode: 404 }))).toBe('destructive')
    const failed = request({ statusCode: null, errorKind: 'blocked_address', error: '10.0.0.1 is not a public address' })
    expect(responded(failed)).toBe(false)
    expect(requestTone(failed)).toBe('warning')
  })
})
