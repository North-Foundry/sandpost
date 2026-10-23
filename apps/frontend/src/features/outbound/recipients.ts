/**
 * Recipient allowlist patterns and forward recipient addresses of SMTP Outbound delivery.
 *
 * Patterns are globs matched against whole envelope recipient addresses: `*` matches any run of
 * characters, `?` exactly one, and letters compare ASCII case-insensitively. An empty allowlist
 * permits no recipient and `*` permits everyone.
 */

/**
 * Most patterns one SMTP Outbound accepts.
 */
export const MAXIMUM_ALLOWED_RECIPIENTS = 100

/**
 * Most recipients one forward action accepts.
 */
export const MAXIMUM_FORWARD_RECIPIENTS = 50

/**
 * Longest accepted address or pattern, matching the SMTP path limit.
 */
const MAXIMUM_LENGTH = 254

/**
 * A plain address: one `@` between a non-empty local part and a dotted or single-label domain.
 */
const ADDRESS = /^[^\s@<>(),;:"[\]\\]+@[A-Za-z0-9](?:[A-Za-z0-9-]*[A-Za-z0-9])?(?:\.[A-Za-z0-9](?:[A-Za-z0-9-]*[A-Za-z0-9])?)*$/

/**
 * How broadly an allowlist permits recipients, for summaries and warnings.
 */
export type AllowlistCoverage = 'nobody' | 'everyone' | 'restricted'

/**
 * Split free text into entries on new lines, commas, semicolons and spaces, dropping blanks and
 * repeated entries (compared ASCII case-insensitively) while keeping the first spelling and order.
 */
export function parseRecipientList(text: string): string[] {
  const seen = new Set<string>()
  const entries: string[] = []
  for (const entry of text.split(/[\s,;]+/)) {
    const value = entry.trim()
    const key = asciiLowercase(value)
    if (!value || seen.has(key)) continue
    seen.add(key)
    entries.push(value)
  }
  return entries
}

/**
 * Render entries one per line for a textarea.
 */
export function formatRecipientList(entries: readonly string[]): string {
  return entries.join('\n')
}

/**
 * Report whether an allowlist entry is acceptable: non-empty, not too long, without spaces, quotes,
 * angle brackets or separators.
 */
export function isValidRecipientPattern(pattern: string): boolean {
  return pattern.length > 0 && pattern.length <= MAXIMUM_LENGTH && !/[\s<>(),;"\\]/.test(pattern)
}

/**
 * Report whether a forward recipient is a plain address without glob characters.
 */
export function isValidRecipientAddress(address: string): boolean {
  return address.length <= MAXIMUM_LENGTH && !/[*?]/.test(address) && ADDRESS.test(address)
}

/**
 * Match a whole address against one glob pattern, ASCII case-insensitively.
 */
export function recipientPatternMatches(pattern: string, address: string): boolean {
  const source = asciiLowercase(pattern)
  const target = asciiLowercase(address)
  let patternIndex = 0
  let targetIndex = 0
  let starIndex = -1
  let resumeIndex = 0
  while (targetIndex < target.length) {
    const character = source[patternIndex]
    if (character === '?' || (character !== undefined && character !== '*' && character === target[targetIndex])) {
      patternIndex += 1
      targetIndex += 1
    } else if (character === '*') {
      starIndex = patternIndex
      resumeIndex = targetIndex
      patternIndex += 1
    } else if (starIndex >= 0) {
      patternIndex = starIndex + 1
      resumeIndex += 1
      targetIndex = resumeIndex
    } else {
      return false
    }
  }
  while (source[patternIndex] === '*') patternIndex += 1
  return patternIndex === source.length
}

/**
 * Report whether any allowlist pattern permits the address; an empty allowlist permits nobody.
 */
export function recipientAllowed(address: string, patterns: readonly string[]): boolean {
  return patterns.some((pattern) => recipientPatternMatches(pattern, address))
}

/**
 * Classify an allowlist: empty permits nobody, a lone-`*` entry everyone, anything else some recipients.
 */
export function allowlistCoverage(patterns: readonly string[]): AllowlistCoverage {
  if (!patterns.length) return 'nobody'
  if (patterns.some((pattern) => /^\*+$/.test(pattern))) return 'everyone'
  return 'restricted'
}

/**
 * Lowercase ASCII letters only, matching the server's comparison.
 */
function asciiLowercase(value: string): string {
  return value.replace(/[A-Z]/g, (letter) => letter.toLowerCase())
}

/**
 * Problems of a forward recipient list against its outbound's allowlist.
 */
export interface ForwardRecipientIssues {
  /**
   * No recipient was entered.
   */
  empty: boolean

  /**
   * More recipients than a forward action accepts.
   */
  tooMany: boolean

  /**
   * Entries that are not plain addresses.
   */
  invalid: string[]

  /**
   * Valid addresses the outbound's allowlist would refuse.
   */
  refused: string[]
}

/**
 * Check forward recipients the way the server does before saving a forward action.
 */
export function forwardRecipientIssues(addresses: readonly string[], allowlist: readonly string[]): ForwardRecipientIssues {
  const invalid = addresses.filter((address) => !isValidRecipientAddress(address))
  return {
    empty: addresses.length === 0,
    tooMany: addresses.length > MAXIMUM_FORWARD_RECIPIENTS,
    invalid,
    refused: addresses.filter((address) => !invalid.includes(address) && !recipientAllowed(address, allowlist)),
  }
}

/**
 * Report whether the issues block saving.
 */
export function hasForwardRecipientIssues(issues: ForwardRecipientIssues): boolean {
  return issues.empty || issues.tooMany || issues.invalid.length > 0 || issues.refused.length > 0
}
