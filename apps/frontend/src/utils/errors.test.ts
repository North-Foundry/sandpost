import { describe, expect, it } from 'vitest'
import { asSentence, describeError } from './errors'

describe('describeError', () => {
  it('prefers the server detail and capitalizes it', () => {
    expect(describeError(Object.assign(new Error('Sandpost API 409: x'), { detail: 'scope still has child scopes' }), 'fallback'))
      .toBe('Scope still has child scopes')
  })

  it('strips the Electron IPC wrapper from native errors', () => {
    expect(describeError(new Error("Error invoking remote method 'sandpost:admin:scopes:save': Error: forbidden"), 'fallback')).toBe('Forbidden')
  })

  it('falls back for values without a message', () => {
    expect(describeError(null, 'Could not save.')).toBe('Could not save.')
  })

  it('formats server fragments as sentences', () => {
    expect(asSentence('127.0.0.1:25 is already in use')).toBe('127.0.0.1:25 is already in use.')
    expect(asSentence('done.')).toBe('Done.')
  })
})
