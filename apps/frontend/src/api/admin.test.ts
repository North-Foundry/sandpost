import { describe, expect, it } from 'vitest'
import { characterIndexForByteOffset, normalizeSmtpAccess, smtpAccessRulesWire, smtpAccessWire } from './admin'

describe('administration DTO helpers', () => {
  it('maps UTF-8 byte offsets to JavaScript string indexes', () => {
    expect(characterIndexForByteOffset('subject == ', 11)).toBe(11)
    expect(characterIndexForByteOffset('subject == "é" and', 15)).toBe(14)
    expect(characterIndexForByteOffset('"😀" x', 6)).toBe(4)
    expect(characterIndexForByteOffset('short', 99)).toBe(5)
  })

  it('converts SMTP access timestamps to milliseconds and blanks to server defaults', () => {
    expect(normalizeSmtpAccess({
      identifier: 'a', name: 'CI', username: 'ci', enabled: false, created_at: 2, last_used_at: 3,
    })).toEqual({
      id: 'a', name: 'CI', username: 'ci', enabled: false, createdAt: 2000, lastUsedAt: 3000,
      requiresEncryption: false, allowedMechanisms: ['plain', 'login'],
    })
    expect(smtpAccessWire({ name: 'CI', username: ' ' })).toEqual({ name: 'CI', username: null })
  })

  it('normalizes access rules in canonical order and encodes them for the server', () => {
    expect(normalizeSmtpAccess({
      identifier: 'a', name: 'CI', username: 'ci', enabled: true, created_at: 2, last_used_at: null,
      requires_encryption: true, allowed_mechanisms: ['login', 'plain'],
    })).toMatchObject({ requiresEncryption: true, allowedMechanisms: ['plain', 'login'] })
    expect(smtpAccessWire({ name: 'CI', requiresEncryption: true, allowedMechanisms: ['login'] }))
      .toEqual({ name: 'CI', username: null, requires_encryption: true, allowed_mechanisms: ['login'] })
    expect(smtpAccessRulesWire({ requiresEncryption: false, allowedMechanisms: ['plain'] }))
      .toEqual({ requires_encryption: false, allowed_mechanisms: ['plain'] })
  })
})
