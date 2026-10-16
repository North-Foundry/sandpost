/**
 * Search-box convenience syntax, normalized to the canonical Sandpost query language.
 *
 * The search box accepts three forms and always produces one canonical DSL string for the server:
 * - canonical DSL (`from.domain == "example.test"`), passed through unchanged;
 * - shortcuts (`from:dev@example.test subject:"password reset" has:attachment`), rewritten as predicates;
 * - plain text, matched literally against all content exactly as before (`content contains "<text>"`).
 *
 * Shortcuts exist only in search. Smart folders and scopes store canonical queries.
 */
import { looksLikeCanonicalQuery, quoteQueryString } from '@/features/query/vocabulary'
import { i18n } from '@/i18n'

export interface SearchShortcut {
  /**
   * The text typed before the colon.
   */
  key: string

  /**
   * Completion inserted for the shortcut; values are filled in by the user unless fixed.
   */
  insert: string
  description: string
}

/**
 * Shortcuts in suggestion order.
 */
export const SEARCH_SHORTCUTS: readonly SearchShortcut[] = [
  { key: 'from', insert: 'from:', description: 'mail.search.shortcut.from' },
  { key: 'to', insert: 'to:', description: 'mail.search.shortcut.to' },
  { key: 'cc', insert: 'cc:', description: 'mail.search.shortcut.cc' },
  { key: 'subject', insert: 'subject:', description: 'mail.search.shortcut.subject' },
  { key: 'has', insert: 'has:attachment', description: 'mail.search.shortcut.has' },
  { key: 'after', insert: 'after:', description: 'mail.search.shortcut.after' },
  { key: 'before', insert: 'before:', description: 'mail.search.shortcut.before' },
]

const SHORTCUT_KEYS = new Set(SEARCH_SHORTCUTS.map((shortcut) => shortcut.key))

/**
 * One recognized shortcut, shown back to the user as a parsed chip.
 */
export interface SearchTerm {
  key: string
  value: string
  negated: boolean
}

export type SearchInterpretation =
  /**
   * Nothing to search yet; `pending` names a shortcut still waiting for its value.
   */
  | { kind: 'empty'; pending?: string }
  | { kind: 'text' | 'shortcuts' | 'canonical'; query: string; terms: SearchTerm[]; text: string }
  | { kind: 'invalid'; message: string }

interface RawToken {
  key: string | null
  value: string
  negated: boolean
}

/**
 * Split input into shortcut and free-text tokens, honoring double-quoted values and phrases.
 */
function tokenize(input: string): RawToken[] {
  const tokens: RawToken[] = []
  let index = 0

  /**
   * Read a quoted value (escapes kept literal) or a bare value up to the next whitespace.
   */
  const readValue = (): string => {
    if (input[index] === '"') {
      const end = input.indexOf('"', index + 1)
      const value = input.slice(index + 1, end < 0 ? undefined : end)
      index = end < 0 ? input.length : end + 1
      return value
    }
    const match = /^\S*/.exec(input.slice(index))![0]
    index += match.length
    return match
  }
  while (index < input.length) {
    if (/\s/.test(input[index]!)) { index += 1; continue }
    const shortcut = /^(-?)([A-Za-z]+):/.exec(input.slice(index))
    if (shortcut && SHORTCUT_KEYS.has(shortcut[2]!.toLowerCase())) {
      index += shortcut[0].length
      tokens.push({ key: shortcut[2]!.toLowerCase(), negated: shortcut[1] === '-', value: readValue() })
    } else {
      tokens.push({ key: null, negated: false, value: readValue() })
    }
  }
  return tokens
}

/**
 * Convert a local calendar date to Unix seconds at local midnight, or null when it is not a real date.
 */
function localMidnightSeconds(value: string): number | null {
  const match = /^(\d{4})-(\d{2})-(\d{2})$/.exec(value)
  if (!match) return null
  const [year, month, day] = [Number(match[1]), Number(match[2]), Number(match[3])]
  const date = new Date(year, month - 1, day)
  if (date.getFullYear() !== year || date.getMonth() !== month - 1 || date.getDate() !== day) return null
  return Math.floor(date.getTime() / 1000)
}

/**
 * Build the canonical predicate for one shortcut, or an error message for an unusable value.
 */
function shortcutPredicate(term: SearchTerm): string | { error: string } {
  const literal = quoteQueryString(term.value)
  switch (term.key) {
    case 'from': return `(from.address contains ${literal} or envelope.from.address contains ${literal})`
    case 'to': return `(to.address contains ${literal} or envelope.to.address contains ${literal})`
    case 'cc': return `cc.address contains ${literal}`
    case 'subject': return `subject contains ${literal}`
    case 'has':
      return /^attachments?$/i.test(term.value) ? 'has_attachments == true' : { error: i18n.global.t('mail.search.invalidHas') }
    case 'after':
    case 'before': {
      const seconds = localMidnightSeconds(term.value)
      if (seconds === null) return { error: i18n.global.t('mail.search.invalidDate', { key: term.key }) }
      return term.key === 'after' ? `received_at >= ${seconds}` : `received_at < ${seconds}`
    }
    default: return { error: i18n.global.t('mail.search.invalidShortcut', { key: term.key }) }
  }
}

/**
 * Interpret search-box input and produce its canonical query.
 */
export function interpretSearch(input: string): SearchInterpretation {
  const source = input.trim()
  if (!source) return { kind: 'empty' }
  if (looksLikeCanonicalQuery(source)) return { kind: 'canonical', query: source, terms: [], text: '' }

  const tokens = tokenize(source)
  const shortcuts = tokens.filter((token) => token.key !== null)
  if (!shortcuts.length) {
    /*
     * Plain text keeps the original literal search; one enclosing pair of quotes searches its inner text literally,
     * which is also the escape hatch for text that would otherwise read as a canonical query.
     */
    const text = /^"([^"]+)"$/.exec(source)?.[1] ?? source
    return { kind: 'text', query: `content contains ${quoteQueryString(text)}`, terms: [], text }
  }

  const terms: SearchTerm[] = []
  const predicates: string[] = []
  let pending: string | undefined
  for (const token of shortcuts) {
    if (!token.value) { pending = token.key!; continue }
    const term = { key: token.key!, value: token.value, negated: token.negated }
    const predicate = shortcutPredicate(term)
    if (typeof predicate !== 'string') return { kind: 'invalid', message: predicate.error }
    terms.push(term)
    predicates.push(term.negated ? `not ${predicate.startsWith('(') ? predicate : `(${predicate})`}` : predicate)
  }
  const text = tokens.filter((token) => token.key === null && token.value).map((token) => token.value).join(' ')
  if (text) predicates.push(`content contains ${quoteQueryString(text)}`)
  if (!predicates.length) return { kind: 'empty', pending }
  return { kind: 'shortcuts', query: predicates.join(' and '), terms, text }
}

/**
 * The whitespace-delimited token under the caret, used to offer shortcut completions.
 */
export interface SearchToken {
  start: number
  end: number
  text: string
}

/**
 * Return the token that ends at the caret, or null when the caret follows whitespace.
 */
export function tokenAtCaret(input: string, caret: number): SearchToken | null {
  const match = /\S+$/.exec(input.slice(0, caret))
  return match ? { start: caret - match[0].length, end: caret, text: match[0] } : null
}

/**
 * Suggest shortcuts whose key extends the typed token, ignoring a leading negation.
 */
export function suggestShortcuts(token: string): SearchShortcut[] {
  const typed = token.replace(/^-/, '').toLowerCase()
  if (!typed || typed.includes('"')) return []
  if (typed.includes(':')) {
    /*
     * Only fixed-value shortcuts can be completed after the colon.
     */
    return SEARCH_SHORTCUTS.filter((shortcut) => shortcut.insert.length > shortcut.key.length + 1
      && shortcut.insert.toLowerCase().startsWith(typed) && shortcut.insert.toLowerCase() !== typed)
  }
  return SEARCH_SHORTCUTS.filter((shortcut) => shortcut.key.startsWith(typed))
}

/**
 * Replace a token with a shortcut completion, keeping its negation, and return the new input and caret.
 */
export function applyShortcut(input: string, token: SearchToken, shortcut: SearchShortcut): { value: string; caret: number } {
  const prefix = token.text.startsWith('-') ? '-' : ''
  /*
   * Fixed shortcuts are complete terms, so a trailing space lets the user continue typing.
   */
  const insert = `${prefix}${shortcut.insert}${shortcut.insert.endsWith(':') ? '' : ' '}`
  const value = input.slice(0, token.start) + insert + input.slice(token.end)
  return { value, caret: token.start + insert.length }
}
