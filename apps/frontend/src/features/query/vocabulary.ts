/**
 * Shared vocabulary of the canonical Sandpost query language.
 *
 * The server compiles and evaluates every query; this module only knows the public grammar well enough to
 * highlight it, describe its fields, recognize canonical input, and quote literals. It never evaluates a query.
 */

export type QueryFieldType = 'mailbox' | 'string' | 'number' | 'boolean'

export interface QueryField {
  name: string
  type: QueryFieldType
  description: string
}

/**
 * Canonical fields in suggestion order. Address aliases (`from`, `to`, …) are accepted but not suggested.
 */
export const QUERY_FIELDS: readonly QueryField[] = [
  { name: 'from.address', type: 'mailbox', description: 'Header sender address' },
  { name: 'from.domain', type: 'mailbox', description: 'Header sender domain' },
  { name: 'to.address', type: 'mailbox', description: 'Header recipient address' },
  { name: 'to.domain', type: 'mailbox', description: 'Header recipient domain' },
  { name: 'cc.address', type: 'mailbox', description: 'Cc address' },
  { name: 'cc.domain', type: 'mailbox', description: 'Cc domain' },
  { name: 'envelope.from.address', type: 'mailbox', description: 'SMTP sender address' },
  { name: 'envelope.from.domain', type: 'mailbox', description: 'SMTP sender domain' },
  { name: 'envelope.to.address', type: 'mailbox', description: 'SMTP recipient address' },
  { name: 'envelope.to.domain', type: 'mailbox', description: 'SMTP recipient domain' },
  { name: 'subject', type: 'string', description: 'Subject (case-sensitive)' },
  { name: 'text', type: 'string', description: 'Plain-text body (case-sensitive)' },
  { name: 'html', type: 'string', description: 'HTML body source (case-sensitive)' },
  { name: 'content', type: 'string', description: 'Any text, address, or header (case-insensitive)' },
  { name: 'message_id', type: 'string', description: 'RFC Message-ID header' },
  { name: 'header["X-Name"]', type: 'string', description: 'Values of one header' },
  { name: 'received_at', type: 'number', description: 'Unix seconds (UTC)' },
  { name: 'size', type: 'number', description: 'Message size in bytes' },
  { name: 'attachment_count', type: 'number', description: 'Number of attachments' },
  { name: 'has_attachments', type: 'boolean', description: 'true or false' },
]

/**
 * Operators available on string and mailbox fields.
 */
export const STRING_OPERATORS = ['==', '!=', 'contains', 'starts_with', 'ends_with', 'matches'] as const

/**
 * Additional ordering operators available on numeric fields.
 */
export const ORDERING_OPERATORS = ['>', '>=', '<', '<='] as const

/**
 * Logical keywords and boolean literals.
 */
export const QUERY_KEYWORDS = ['and', 'or', 'not', 'true', 'false'] as const

const WORD_OPERATORS = new Set(['contains', 'starts_with', 'ends_with', 'matches'])
const KEYWORDS = new Set<string>(QUERY_KEYWORDS)

/**
 * Every field spelling the compiler accepts, including the address aliases.
 */
const FIELD_SOURCE = String.raw`(?:envelope\.(?:from|to)|from|to|cc)(?:\.(?:address|domain))?|subject|text|html|content|message_id|received_at|size|attachment_count|has_attachments`
const FIELD_NAME = new RegExp(`^(?:${FIELD_SOURCE})$`, 'i')

/**
 * A field (or `header[...]`) immediately followed by a comparison operator.
 */
const CANONICAL_COMPARISON = new RegExp(
  String.raw`(?:^|[\s(])(?:${FIELD_SOURCE}|header\s*\[\s*"(?:[^"\\]|\\.)*"\s*\])(?:\s*(?:==|!=|>=|<=|<|>)|\s+(?:contains|starts_with|ends_with|matches)\b)`,
  'i',
)

/**
 * Quote a literal as a DSL string, escaping only the characters the lexer treats specially.
 */
export function quoteQueryString(value: string): string {
  return `"${value.replace(/[\\"]/g, '\\$&')}"`
}

/**
 * Report whether input contains a canonical comparison, which marks it as DSL rather than search text.
 */
export function looksLikeCanonicalQuery(source: string): boolean {
  return CANONICAL_COMPARISON.test(source)
}

/**
 * Report whether an identifier names a canonical field or alias (case-insensitive).
 */
export function isQueryField(identifier: string): boolean {
  return FIELD_NAME.test(identifier)
}

/**
 * Classify an identifier-like word as a keyword, word operator, field, or plain text.
 */
export function classifyQueryWord(word: string): QueryTokenKind {
  const lower = word.toLowerCase()
  if (KEYWORDS.has(lower)) return 'keyword'
  if (WORD_OPERATORS.has(lower)) return 'operator'
  return lower === 'header' || isQueryField(lower) ? 'field' : 'plain'
}

export type QueryTokenKind = 'field' | 'operator' | 'keyword' | 'string' | 'number' | 'punctuation' | 'plain'

export interface QueryToken {
  text: string
  kind: QueryTokenKind
}

/**
 * Split source into display tokens for syntax highlighting.
 *
 * The concatenated token text always equals the source, including unterminated strings and unknown characters,
 * so a highlighted mirror stays aligned with the editable text.
 */
export function highlightTokens(source: string): QueryToken[] {
  const tokens: QueryToken[] = []

  /**
   * Appends a token, merging adjacent plain text to keep source coverage compact.
   */
  const push = (text: string, kind: QueryTokenKind) => {
    const previous = tokens.at(-1)
    if (previous && previous.kind === kind && kind === 'plain') previous.text += text
    else tokens.push({ text, kind })
  }
  let index = 0
  while (index < source.length) {
    const rest = source.slice(index)
    const string = /^"(?:[^"\\]|\\[\s\S])*(?:"|$)/.exec(rest)
    const operator = /^(?:==|!=|>=|<=|<|>)/.exec(rest)
    const number = /^-?\d+/.exec(rest)
    const word = /^[A-Za-z_][A-Za-z0-9_.]*/.exec(rest)
    if (string) push(string[0], 'string')
    else if (operator) push(operator[0], 'operator')
    else if (number) push(number[0], 'number')
    else if (word) push(word[0], classifyQueryWord(word[0]))
    else if ('()[]'.includes(rest[0]!)) push(rest[0]!, 'punctuation')
    else push(rest[0]!, 'plain')
    index += (string ?? operator ?? number ?? word)?.[0].length ?? 1
  }
  return tokens
}
