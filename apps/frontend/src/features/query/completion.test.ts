import { describe, expect, it } from 'vitest'
import { completionContext, describeTokenAt, suggestionsFor } from './completion'
import { hasAnyInequality, queryDiagnostics } from './editor'

/**
 * Return the parser expectation at the end of the supplied query text.
 */
function expectationAtEnd(source: string) {
  return completionContext(source, source.length).expectation
}

/**
 * Return completion labels available at the end of the supplied query text.
 */
function labelsAtEnd(source: string): string[] {
  return suggestionsFor(expectationAtEnd(source)).map((suggestion) => suggestion.label)
}

describe('query completion', () => {
  it('expects an operand at the start and after connectors, negation, and open parentheses', () => {
    for (const source of ['', '   ', '(', 'subject == "x" and ', 'size > 1 or ', 'not ', 'not (']) {
      expect(expectationAtEnd(source)).toEqual({ kind: 'operand' })
    }
    expect(labelsAtEnd('')).toContain('envelope.to.domain')
    expect(labelsAtEnd('')).toContain('not')
  })

  it('offers only the operators the compiler accepts for the field type', () => {
    expect(expectationAtEnd('from.domain ')).toEqual({ kind: 'operator', type: 'mailbox' })
    expect(labelsAtEnd('from.domain ')).not.toContain('>')
    expect(labelsAtEnd('subject ')).toContain('>=')
    expect(labelsAtEnd('size ')).toEqual(['==', '!=', '>', '>=', '<', '<='])
    expect(labelsAtEnd('has_attachments ')).toEqual(['==', '!='])
    expect(expectationAtEnd('header["X-App"] ')).toEqual({ kind: 'operator', type: 'string' })
    /*
     * Aliases select their address field.
     */
    expect(expectationAtEnd('envelope.to ')).toEqual({ kind: 'operator', type: 'mailbox' })
  })

  it('keeps the typed prefix replaceable, including symbolic operators', () => {
    expect(completionContext('subject == "x" and envelope.t', 29)).toEqual({ from: 19, prefix: 'envelope.t', expectation: { kind: 'operand' } })
    expect(completionContext('size >', 6)).toEqual({ from: 5, prefix: '>', expectation: { kind: 'operator', type: 'number' } })
  })

  it('suggests values for booleans and connectors after a complete comparison', () => {
    expect(labelsAtEnd('has_attachments == ')).toEqual(['true', 'false'])
    expect(labelsAtEnd('subject contains ')).toEqual([])
    for (const source of ['subject == "x" ', 'size >= 10 ', '(to.domain == "a.test") ', 'has_attachments == true ']) {
      expect(labelsAtEnd(source)).toEqual(['and', 'or'])
    }
  })

  it('never completes inside a string literal', () => {
    expect(expectationAtEnd('subject == "envel')).toEqual({ kind: 'none' })
    expect(expectationAtEnd('header["X-')).toEqual({ kind: 'none' })
  })

  it('opens quotes for text values and brackets for header lookups', () => {
    const contains = suggestionsFor({ kind: 'operator', type: 'string' }).find((suggestion) => suggestion.label === 'contains')
    expect(contains).toMatchObject({ insert: 'contains ', open: '"' })
    const size = suggestionsFor({ kind: 'operator', type: 'number' }).find((suggestion) => suggestion.label === '>=')
    expect(size).toMatchObject({ insert: '>= ', open: undefined })
    const header = suggestionsFor({ kind: 'operand' }).find((suggestion) => suggestion.label.startsWith('header['))
    expect(header).toMatchObject({ insert: 'header', open: '["' })
  })

  it('documents fields, aliases, operators, and keywords under the pointer', () => {
    const source = 'from contains "a" and size >= 1'
    expect(describeTokenAt(source, 1)).toMatchObject({ from: 0, to: 4, detail: 'address', info: 'Header sender address (short for from.address)' })
    expect(describeTokenAt(source, 7)).toMatchObject({ title: 'contains', detail: 'operator' })
    expect(describeTokenAt(source, 19)).toMatchObject({ title: 'and', detail: 'keyword' })
    expect(describeTokenAt(source, 15)).toBeNull()
  })
})

describe('query diagnostics', () => {
  it('underlines the token at the server position, measured in UTF-8 bytes', () => {
    /*
     * "é" is two bytes, so byte 18 is the character index 17 where `bogus` starts.
     */
    const source = 'subject == "é" or bogus'
    expect(queryDiagnostics(source, { position: 19, message: 'unknown field' })).toEqual([
      { from: 18, to: 23, severity: 'error', message: 'Unknown field.' },
    ])
    expect(queryDiagnostics('subject ==', { position: 10, message: 'expected a value' })[0]).toMatchObject({ from: 9, to: 10 })
  })

  it('warns where != compares a collection', () => {
    const source = 'subject != "a" and to.domain != "b.test"'
    expect(hasAnyInequality(source)).toBe(true)
    expect(queryDiagnostics(source, null)).toEqual([expect.objectContaining({ from: 29, to: 31, severity: 'warning' })])
    expect(hasAnyInequality('subject != "a"')).toBe(false)
  })
})
