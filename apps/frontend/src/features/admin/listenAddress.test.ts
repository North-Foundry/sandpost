import { describe, expect, it } from 'vitest'
import { configuredListenerAddress, connectionAddress, splitAddress } from './listenAddress'

describe('listener address fields', () => {
  it('shows the raw configured bind, including wildcard hosts and port zero', () => {
    expect(configuredListenerAddress({
      listenAddress: '127.0.0.1:32000',
      listener: {
        configuredAddress: '0.0.0.0:0', boundAddress: '0.0.0.0:32000', source: 'configuration',
        state: 'listening', error: null, authenticationRequired: true, encryption: 'none',
      },
    })).toBe('0.0.0.0:0')
  })

  it('splits runtime socket addresses into host and port', () => {
    expect(splitAddress('0.0.0.0:2525')).toEqual({ host: '0.0.0.0', port: '2525' })
    expect(splitAddress('[::1]:25')).toEqual({ host: '::1', port: '25' })
    expect(splitAddress(null)).toEqual({ host: '127.0.0.1', port: '' })
  })

  it('turns wildcard binds into an address SMTP clients can reach', () => {
    expect(connectionAddress('127.0.0.1:1025', 'mail.example.test')).toEqual({ host: '127.0.0.1', port: '1025' })
    expect(connectionAddress('0.0.0.0:2525', 'mail.example.test')).toEqual({ host: 'mail.example.test', port: '2525' })
    expect(connectionAddress('[::]:2525', '')).toEqual({ host: '::1', port: '2525' })
    expect(connectionAddress('0.0.0.0:2525', 'app')).toEqual({ host: '127.0.0.1', port: '2525' })
  })
})
