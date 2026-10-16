import { describe, expect, it } from 'vitest'
import { highlightTokens, looksLikeCanonicalQuery } from './vocabulary'

describe('query vocabulary', () => {
  it('recognizes canonical comparisons without mistaking search text for them', () => {
    expect(looksLikeCanonicalQuery('subject contains "x"')).toBe(true)
    expect(looksLikeCanonicalQuery('(envelope.to.domain=="a.test")')).toBe(true)
    expect(looksLikeCanonicalQuery('header["X-App"] != "x"')).toBe(true)
    expect(looksLikeCanonicalQuery('subject line contains typo')).toBe(false)
    expect(looksLikeCanonicalQuery('from:dev@example.test')).toBe(false)
    expect(looksLikeCanonicalQuery('subjectcontains x')).toBe(false)
  })

  it('tokenizes for highlighting without losing or reordering source text', () => {
    const source = 'not (from.domain == "a.test" or size >= 10) and header["X"] contains "unterminated'
    const tokens = highlightTokens(source)
    expect(tokens.map((token) => token.text).join('')).toBe(source)
    expect(tokens.filter((token) => token.kind === 'field').map((token) => token.text)).toEqual(['from.domain', 'size', 'header'])
    expect(tokens.filter((token) => token.kind === 'keyword').map((token) => token.text)).toEqual(['not', 'or', 'and'])
    expect(tokens.at(-1)).toEqual({ text: '"unterminated', kind: 'string' })
  })
})
