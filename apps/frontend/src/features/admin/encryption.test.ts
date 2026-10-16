import { describe, expect, it } from 'vitest'
import type { ListenerEncryption } from '@/api/contracts'
import { connectionProtection, credentialProtection } from './encryption'

describe('SMTP encryption descriptions', () => {
  it('describes every encryption mode and warns only when credentials may travel in clear text', () => {
    const modes: ListenerEncryption[] = ['none', 'startTlsOptional', 'startTlsRequired', 'implicit']
    for (const mode of modes) {
      expect(connectionProtection(mode)).toMatch(/\.$/)
      expect(credentialProtection(mode).length).toBeGreaterThan(0)
    }
    expect(credentialProtection('none')).toContain('unencrypted')
    expect(credentialProtection('startTlsOptional')).toContain('clear text')
    expect(credentialProtection('startTlsRequired')).not.toContain('unencrypted')
    expect(connectionProtection('implicit')).toContain('TLS')
  })
})
