import { describe, expect, it } from 'vitest'
import { StringStream } from '@codemirror/language'
import { insideCondition, scriptDiagnostics, scriptToken } from './scriptEditor'

/**
 * Tokenize one line and return each token's text and kind.
 */
function tokens(line: string, state = { quote: null as string | null, comment: false, condition: 0, afterIf: false }) {
  const stream = new StringStream(line, 4, 4)
  const result: [string, string | null][] = []
  while (!stream.eol()) {
    stream.start = stream.pos
    const kind = scriptToken(stream, state)
    const text = line.slice(stream.start, stream.pos)
    if (text.trim()) result.push([text, kind])
  }
  return result
}

describe('PersonaScript highlighting', () => {
  it('classifies statements, durations, arguments, and comments', () => {
    expect(tokens('wait 5s; click link(text: "Go"); // done')).toEqual([
      ['wait', 'keyword'], ['5s', 'number'], [';', 'punctuation'], ['click', 'keyword'], ['link', 'keyword'],
      ['(', 'punctuation'], ['text', 'field'], [':', 'punctuation'], ['"Go"', 'string'], [')', 'punctuation'],
      [';', 'punctuation'], ['// done', 'comment'],
    ])
  })

  it('highlights query words inside conditions only', () => {
    const highlighted = tokens('if (subject contains "x" and (size > 1)) { open; }')
    expect(highlighted).toContainEqual(['subject', 'field'])
    expect(highlighted).toContainEqual(['contains', 'operator'])
    expect(highlighted).toContainEqual(['and', 'keyword'])
    expect(highlighted).toContainEqual(['1', 'number'])
    expect(highlighted.at(-2)).toEqual([';', 'punctuation'])
    expect(tokens('subject;')).toEqual([['subject', null], [';', 'punctuation']])
  })

  it('locates conditions for completion', () => {
    const source = 'open; if (subject == "a ) b" and ('
    expect(insideCondition(source, source.length)).toBe(true)
    expect(insideCondition('if (true) { ', 12)).toBe(false)
    expect(insideCondition('// if (\nopen;', 13)).toBe(false)
  })

  it('maps byte diagnostics to the offending word', () => {
    const source = 'open;\nwait 5x;'
    const [diagnostic] = scriptDiagnostics(source, { message: 'unknown duration unit `x`', position: 12, line: 2, column: 7 })
    expect(source.slice(diagnostic!.from, diagnostic!.to)).toBe('x')
    expect(scriptDiagnostics(source, null)).toEqual([])
    const unicode = 'é jump;'
    const [shifted] = scriptDiagnostics(unicode, { message: 'unknown', position: 3, line: 1, column: 3 })
    expect(unicode.slice(shifted!.from, shifted!.to)).toBe('jump')
  })
})
