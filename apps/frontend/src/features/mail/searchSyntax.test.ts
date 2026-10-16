import { describe, expect, it } from 'vitest'
import { applyShortcut, interpretSearch, suggestShortcuts, tokenAtCaret } from './searchSyntax'

describe('search syntax normalization', () => {
  it('keeps plain text as the original literal content search', () => {
    expect(interpretSearch('  password reset ')).toEqual({
      kind: 'text', query: 'content contains "password reset"', terms: [], text: 'password reset',
    })
    expect(interpretSearch('say "hi" \\ there')).toMatchObject({ query: 'content contains "say \\"hi\\" \\\\ there"' })
    expect(interpretSearch('https://example.test/reset')).toMatchObject({ kind: 'text' })
    expect(interpretSearch('"subject contains x"')).toMatchObject({ kind: 'text', query: 'content contains "subject contains x"' })
  })

  it('passes canonical queries through unchanged', () => {
    for (const query of [
      'from.address == "dev@example.test"',
      '(to.domain == "a.test" or cc.domain == "b.test") and size > 10',
      'not (subject contains "x")',
      'header["X-App"] == "payments"',
      'envelope.to matches "*@sandpost.local"',
    ]) expect(interpretSearch(query)).toEqual({ kind: 'canonical', query, terms: [], text: '' })
  })

  it('rewrites shortcuts into canonical predicates joined with free text', () => {
    expect(interpretSearch('from:dev@example.test subject:"password reset" invoice')).toEqual({
      kind: 'shortcuts',
      query: '(from.address contains "dev@example.test" or envelope.from.address contains "dev@example.test")'
        + ' and subject contains "password reset" and content contains "invoice"',
      terms: [
        { key: 'from', value: 'dev@example.test', negated: false },
        { key: 'subject', value: 'password reset', negated: false },
      ],
      text: 'invoice',
    })
    expect(interpretSearch('TO:qa@sandpost.local -has:attachment')).toMatchObject({
      query: '(to.address contains "qa@sandpost.local" or envelope.to.address contains "qa@sandpost.local")'
        + ' and not (has_attachments == true)',
    })
    expect(interpretSearch('-from:noreply')).toMatchObject({
      query: 'not (from.address contains "noreply" or envelope.from.address contains "noreply")',
    })
  })

  it('converts dates to local-midnight Unix seconds', () => {
    const midnight = Math.floor(new Date(2026, 9, 7).getTime() / 1000)
    expect(interpretSearch('after:2026-10-07')).toMatchObject({ query: `received_at >= ${midnight}` })
    expect(interpretSearch('before:2026-10-07')).toMatchObject({ query: `received_at < ${midnight}` })
    expect(interpretSearch('after:2026-02-30')).toEqual({ kind: 'invalid', message: 'after: expects a date like 2026-10-07.' })
  })

  it('waits for a shortcut value and rejects unsupported fixed values', () => {
    expect(interpretSearch('from:')).toEqual({ kind: 'empty', pending: 'from' })
    expect(interpretSearch('has:link')).toEqual({ kind: 'invalid', message: 'has: supports only has:attachment.' })
    expect(interpretSearch('   ')).toEqual({ kind: 'empty' })
  })

  it('completes shortcuts at the caret while preserving negation', () => {
    const input = 'invoice -fr'
    const token = tokenAtCaret(input, input.length)!
    expect(token).toEqual({ start: 8, end: 11, text: '-fr' })
    const [from] = suggestShortcuts(token.text)
    expect(from?.key).toBe('from')
    expect(applyShortcut(input, token, from!)).toEqual({ value: 'invoice -from:', caret: 14 })
    expect(suggestShortcuts('has:').map((shortcut) => shortcut.insert)).toEqual(['has:attachment'])
    expect(suggestShortcuts('from:')).toEqual([])
    expect(tokenAtCaret('from ', 5)).toBeNull()
  })
})
