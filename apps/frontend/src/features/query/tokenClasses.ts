import type { QueryTokenKind } from './vocabulary'

/**
 * Utility classes that color each query token kind, shared by static highlighting and the editor.
 */
export const QUERY_TOKEN_CLASSES: Record<QueryTokenKind, string> = {
  field: 'text-syntax-field',
  operator: 'text-syntax-operator',
  keyword: 'text-syntax-keyword font-medium',
  string: 'text-syntax-string',
  number: 'text-syntax-number',
  punctuation: 'text-muted-foreground',
  plain: '',
}
