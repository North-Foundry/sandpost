import { describe, expect, it } from 'vitest'
import type { SmtpOutbound } from '@/api/contracts'
import {
  emptyOutboundForm,
  hasAdvancedSettings,
  outboundFormFrom,
  outboundFormProblems,
  outboundFormWarnings,
  outboundInputFromForm,
  portAfterSecurityChange,
} from './outboundForm'

const stored: SmtpOutbound = {
  id: 'outbound-1', name: 'Pippo', host: 'smtp.example.com', port: 2525, security: 'tls', authenticationType: 'login',
  username: 'user', passwordConfigured: true, enabled: false, overrideFrom: null, envelopeFrom: null,
  connectTimeoutSeconds: null, smtpTimeoutSeconds: 45, tlsVerify: true, heloName: null, maxConnections: null,
  allowedRecipients: ['*@example.com', 'qa@company.test'], routingActionCount: 0, createdAt: 0, updatedAt: 0,
}

describe('SMTP Outbound form', () => {
  /**
   * Editing starts with a blank password and the allowlist one pattern per line.
   */
  it('copies a stored outbound without its password', () => {
    const form = outboundFormFrom(stored)
    expect(form).toMatchObject({ password: '', port: '2525', smtpTimeout: '45', connectTimeout: '', allowedRecipients: '*@example.com\nqa@company.test' })
    expect(hasAdvancedSettings(form)).toBe(true)
    expect(hasAdvancedSettings(emptyOutboundForm())).toBe(false)
  })

  /**
   * A blank password is valid only when one is stored.
   */
  it('requires a password only without a stored one', () => {
    const form = { ...emptyOutboundForm(), name: 'Relay', host: 'smtp.example.com', username: 'user' }
    expect(outboundFormProblems(form).password?.key).toBe('outbound.form.error.passwordRequired')
    expect(outboundFormProblems(form, { passwordConfigured: true })).toEqual({})
    expect(outboundFormProblems({ ...form, password: 'secret' }, { passwordConfigured: false, credentialsAvailable: false }).password?.key)
      .toBe('outbound.form.error.credentialsUnavailable')
    expect(outboundFormProblems({ ...form, authenticationType: 'none', username: '' })).toEqual({})
  })

  /**
   * Numbers, hosts, addresses and patterns are validated before sending.
   */
  it('validates ranges, hosts, addresses and patterns', () => {
    const form = {
      ...emptyOutboundForm(), name: 'Relay', host: 'https://smtp.example.com', port: '70000', authenticationType: 'none' as const,
      connectTimeout: '0', smtpTimeout: '601', maxConnections: '2.5', overrideFrom: 'nobody', allowedRecipients: 'ok@example.com\n<bad>',
    }
    const problems = outboundFormProblems(form)
    expect(Object.keys(problems).sort()).toEqual(['allowedRecipients', 'connectTimeout', 'host', 'maxConnections', 'overrideFrom', 'port', 'smtpTimeout'])
    expect(problems.port).toEqual({ key: 'outbound.form.error.range', params: { minimum: 1, maximum: 65535 } })
    expect(problems.allowedRecipients?.params).toEqual({ patterns: '<bad>' })
    expect(outboundFormProblems({ ...emptyOutboundForm(), name: '', host: '' })).toMatchObject({
      name: { key: 'outbound.form.error.nameRequired' }, host: { key: 'outbound.form.error.hostRequired' },
    })
  })

  /**
   * Warn about clear-text credentials, unverified certificates and the empty allowlist.
   */
  it('warns about risky settings', () => {
    expect(outboundFormWarnings({ ...emptyOutboundForm(), security: 'none' })).toEqual({
      plaintextCredentials: true, certificateUnverified: false, nobodyAllowed: true, everyoneAllowed: false,
    })
    expect(outboundFormWarnings({ ...emptyOutboundForm(), tlsVerify: false, allowedRecipients: '*' })).toEqual({
      plaintextCredentials: false, certificateUnverified: true, nobodyAllowed: false, everyoneAllowed: true,
    })
  })

  /**
   * Encode numbers, blanks as server defaults, the empty allowlist and only a typed password.
   */
  it('builds client input from the form', () => {
    const form = { ...outboundFormFrom(stored), allowedRecipients: '', enabled: true }
    expect(outboundInputFromForm(form)).toEqual({
      name: 'Pippo', host: 'smtp.example.com', port: 2525, security: 'tls', authenticationType: 'login', username: 'user',
      enabled: true, overrideFrom: null, envelopeFrom: null, connectTimeoutSeconds: null, smtpTimeoutSeconds: 45, tlsVerify: true,
      heloName: null, maxConnections: null, allowedRecipients: [],
    })
    expect(outboundInputFromForm({ ...form, password: 'new' }).password).toBe('new')
    expect(outboundInputFromForm({ ...form, authenticationType: 'none', password: 'new' })).toMatchObject({ username: null })
    expect(outboundInputFromForm({ ...form, authenticationType: 'none', password: 'new' })).not.toHaveProperty('password')
  })

  /**
   * The port follows the security mode only while it is that mode's default.
   */
  it('updates default ports when security changes', () => {
    expect(portAfterSecurityChange('587', 'starttls', 'tls')).toBe('465')
    expect(portAfterSecurityChange('2525', 'starttls', 'tls')).toBe('2525')
    expect(portAfterSecurityChange('', 'tls', 'none')).toBe('25')
  })
})
