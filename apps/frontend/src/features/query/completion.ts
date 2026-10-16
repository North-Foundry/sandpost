/**
 * Context-aware completion and documentation for the canonical query language.
 *
 * Like the rest of the query vocabulary this only reads the public grammar: it decides what may come next at a
 * caret (an operand, an operator for the preceding field's type, a value, or a connector) so the editor can offer
 * the right suggestions. The server remains the only validator.
 */
import { ORDERING_OPERATORS, QUERY_FIELDS, STRING_OPERATORS, highlightTokens, type QueryFieldType, type QueryToken } from './vocabulary'
import { i18n } from '@/i18n'

/**
 * What the grammar allows at a caret, judged from the tokens before it.
 */
export type QueryExpectation =
  | { kind: 'operand' }
  | { kind: 'operator'; type: QueryFieldType }
  | { kind: 'value'; type: QueryFieldType }
  | { kind: 'connector' }
  | { kind: 'none' }

/**
 * A suggestion the editor can render and insert.
 */
export interface QuerySuggestion {
  label: string
  kind: 'field' | 'operator' | 'keyword' | 'value'

  /**
   * Short type shown beside the label, for suggestions that mix kinds such as fields and `not`.
   */
  detail?: string

  /**
   * One-line explanation shown next to the list.
   */
  info: string

  /**
   * Text inserted in place of the typed prefix; defaults to the label.
   */
  insert?: string

  /**
   * Brackets or quotes opened after `insert`, auto-closed with the caret inside them.
   */
  open?: string

  /**
   * Whether the next suggestions should open right after inserting this one.
   */
  chain?: boolean
}

/**
 * The completion context at a caret: the replaceable prefix and what may be typed there.
 */
export interface QueryCompletionContext {
  from: number
  prefix: string
  expectation: QueryExpectation
}

/**
 * A source token with its offset, for context and hover lookups.
 */
export interface PositionedQueryToken extends QueryToken {
  from: number
  to: number
}

const TYPE_KEYS: Record<QueryFieldType, string> = { mailbox: 'address', string: 'text', number: 'number', boolean: 'boolean' }
const OPERATOR_KEYS: Record<string, string> = {
  '==': 'equals', '!=': 'differs', contains: 'contains', starts_with: 'startsWith', ends_with: 'endsWith', matches: 'matches',
  '>': 'greaterThan', '>=': 'greaterThanOrEqual', '<': 'lessThan', '<=': 'lessThanOrEqual',
}
const KEYWORD_KEYS: Record<string, string> = { and: 'and', or: 'or', not: 'not', true: 'true', false: 'false' }

/**
 * Translate a query message when it is used so editor help follows the active locale.
 */
function translate(key: string, params?: Record<string, string>): string {
  return i18n.global.t(key, params ?? {})
}

/**
 * Translate the concise display label for a field type.
 */
function typeLabel(type: QueryFieldType): string {
  return translate(`query.completion.types.${TYPE_KEYS[type]}`)
}

/**
 * Translate a field description using the canonical field name as its stable key.
 */
function fieldDescription(name: string): string {
  const key = name.startsWith('header[') ? 'header' : name.toLowerCase().replaceAll('.', '_')
  return translate(`query.completion.fields.${key}`)
}

/**
 * Operators the compiler accepts for each field type.
 */
const OPERATORS_BY_TYPE: Record<QueryFieldType, readonly string[]> = {
  mailbox: STRING_OPERATORS,
  string: [...STRING_OPERATORS, ...ORDERING_OPERATORS],
  number: ['==', '!=', ...ORDERING_OPERATORS],
  boolean: ['==', '!='],
}

/**
 * Address aliases select their `.address` field.
 */
const ALIASES: Record<string, string> = {
  from: 'from.address',
  to: 'to.address',
  cc: 'cc.address',
  'envelope.from': 'envelope.from.address',
  'envelope.to': 'envelope.to.address',
}

/**
 * Look up a field by any accepted spelling, including aliases and `header`.
 */
function fieldInfo(name: string) {
  const lower = name.toLowerCase()
  if (lower === 'header') return QUERY_FIELDS.find((field) => field.name.startsWith('header['))
  return QUERY_FIELDS.find((field) => field.name === (ALIASES[lower] ?? lower))
}

/**
 * Split source into highlight tokens with their offsets.
 */
export function positionedTokens(source: string): PositionedQueryToken[] {
  let offset = 0
  return highlightTokens(source).map((token) => {
    const positioned = { ...token, from: offset, to: offset + token.text.length }
    offset = positioned.to
    return positioned
  })
}

/**
 * Tokens that carry grammar; whitespace and unknown characters are dropped.
 */
function significant(tokens: PositionedQueryToken[]): PositionedQueryToken[] {
  return tokens.filter((token) => token.kind !== 'plain' || token.text.trim() !== '')
}

/**
 * The type of the field ending at `index`, whether a plain field or a `header["…"]` lookup.
 */
function fieldTypeEndingAt(tokens: PositionedQueryToken[], index: number): QueryFieldType | null {
  const token = tokens[index]
  if (!token) return null
  if (token.kind === 'field') return token.text.toLowerCase() === 'header' ? null : fieldInfo(token.text)?.type ?? null
  const isHeaderLookup = token.text === ']' && tokens[index - 1]?.kind === 'string' && tokens[index - 2]?.text === '['
    && tokens[index - 3]?.text.toLowerCase() === 'header'
  return isHeaderLookup ? 'string' : null
}

/**
 * Decide what the grammar allows after the given tokens.
 */
function expectationAfter(tokens: PositionedQueryToken[]): QueryExpectation {
  const index = tokens.length - 1
  const last = tokens[index]
  if (!last) return { kind: 'operand' }
  const lower = last.text.toLowerCase()
  if (last.text === '(' || lower === 'and' || lower === 'or' || lower === 'not') return { kind: 'operand' }
  if (last.kind === 'string' || last.kind === 'number' || last.text === ')' || lower === 'true' || lower === 'false') {
    return { kind: 'connector' }
  }
  if (last.kind === 'operator') {
    return { kind: 'value', type: fieldTypeEndingAt(tokens, index - 1) ?? 'string' }
  }
  const fieldType = fieldTypeEndingAt(tokens, index)
  return fieldType ? { kind: 'operator', type: fieldType } : { kind: 'none' }
}

/**
 * Report whether a caret sits inside an open string literal.
 */
function insideString(before: string): boolean {
  return (before.replace(/\\[\s\S]/g, '').match(/"/g)?.length ?? 0) % 2 === 1
}

/**
 * Find the replaceable prefix at a caret and what may be typed there.
 */
export function completionContext(source: string, caret: number): QueryCompletionContext {
  const before = source.slice(0, caret)
  if (insideString(before)) return { from: caret, prefix: '', expectation: { kind: 'none' } }
  const prefix = /(?:[A-Za-z_][A-Za-z0-9_.]*|[=!<>]+)$/.exec(before)?.[0] ?? ''
  const from = caret - prefix.length
  return { from, prefix, expectation: expectationAfter(significant(positionedTokens(source.slice(0, from)))) }
}

/**
 * List what may be inserted for an expectation, before filtering by the typed prefix.
 */
export function suggestionsFor(expectation: QueryExpectation): QuerySuggestion[] {
  switch (expectation.kind) {
    case 'operand':
      return [
        ...QUERY_FIELDS.map((field): QuerySuggestion => field.name.startsWith('header[')
          ? { label: field.name, kind: 'field', detail: typeLabel(field.type), info: fieldDescription(field.name), insert: 'header', open: '["' }
          : { label: field.name, kind: 'field', detail: typeLabel(field.type), info: fieldDescription(field.name), insert: `${field.name} `, chain: true }),
        { label: 'not', kind: 'keyword', detail: translate('query.completion.detail.keyword'), info: translate('query.completion.keywords.not'), insert: 'not ', chain: true },
      ]
    case 'operator':
      return OPERATORS_BY_TYPE[expectation.type].map((operator): QuerySuggestion => {
        const quoted = expectation.type === 'mailbox' || expectation.type === 'string'
        return {
          label: operator,
          kind: 'operator',
          info: translate(`query.completion.operators.${OPERATOR_KEYS[operator]}`),
          insert: `${operator} `,
          open: quoted ? '"' : undefined,
          chain: expectation.type === 'boolean',
        }
      })
    case 'value':
      return expectation.type === 'boolean'
        ? ['true', 'false'].map((value) => ({ label: value, kind: 'value', info: translate(`query.completion.keywords.${KEYWORD_KEYS[value]}`) }))
        : []
    case 'connector':
      return ['and', 'or'].map((word) => ({ label: word, kind: 'keyword', info: translate(`query.completion.keywords.${KEYWORD_KEYS[word]}`), insert: `${word} `, chain: true }))
    case 'none':
      return []
  }
}

/**
 * Documentation for the field, operator, or keyword token covering an offset, for hover help.
 */
export function describeTokenAt(source: string, offset: number): { from: number; to: number; title: string; detail: string; info: string } | null {
  const token = positionedTokens(source).find((candidate) => candidate.from <= offset && offset < candidate.to)
  if (!token) return null
  const lower = token.text.toLowerCase()
  if (token.kind === 'field') {
    const field = fieldInfo(lower)
    if (!field) return null
    const alias = ALIASES[lower]
    return {
      ...token,
      title: token.text,
      detail: typeLabel(field.type),
      info: alias
        ? translate('query.completion.alias', { description: fieldDescription(field.name), alias })
        : fieldDescription(field.name),
    }
  }
  if (token.kind === 'operator' && OPERATOR_KEYS[lower]) {
    return { ...token, title: token.text, detail: translate('query.completion.detail.operator'), info: translate(`query.completion.operators.${OPERATOR_KEYS[lower]}`) }
  }
  if (token.kind === 'keyword' && KEYWORD_KEYS[lower]) {
    return { ...token, title: token.text, detail: translate('query.completion.detail.keyword'), info: translate(`query.completion.keywords.${KEYWORD_KEYS[lower]}`) }
  }
  return null
}
