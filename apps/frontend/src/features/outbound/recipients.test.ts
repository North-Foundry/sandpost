import { describe, expect, it } from 'vitest'
import {
  allowlistCoverage,
  forwardRecipientIssues,
  hasForwardRecipientIssues,
  isValidRecipientAddress,
  isValidRecipientPattern,
  parseRecipientList,
  recipientAllowed,
  recipientPatternMatches,
} from './recipients'

describe('outbound recipient patterns', () => {
  /**
   * Split on lines, commas, semicolons and spaces, dropping blanks and case-insensitive repeats.
   */
  it('parses free-text recipient lists', () => {
    expect(parseRecipientList(' *@Example.com\n\nqa@company.test, dev@company.test;*@example.COM  \n')).toEqual([
      '*@Example.com', 'qa@company.test', 'dev@company.test',
    ])
    expect(parseRecipientList('   \n ')).toEqual([])
  })

  /**
   * Match whole addresses with * and ?, ignoring ASCII case.
   */
  it('matches glob patterns against whole addresses', () => {
    expect(recipientPatternMatches('*', 'anyone@anywhere.test')).toBe(true)
    expect(recipientPatternMatches('*@Example.com', 'QA@example.COM')).toBe(true)
    expect(recipientPatternMatches('*@example.com', 'qa@example.com.evil')).toBe(false)
    expect(recipientPatternMatches('qa+?@example.com', 'qa+1@example.com')).toBe(true)
    expect(recipientPatternMatches('qa+?@example.com', 'qa+12@example.com')).toBe(false)
    expect(recipientPatternMatches('qa*@*.test', 'qa-team@ci.company.test')).toBe(true)
    expect(recipientPatternMatches('qa@company.test', 'qa@company.test')).toBe(true)
    expect(recipientPatternMatches('qa@company.test', 'xqa@company.test')).toBe(false)
  })

  /**
   * An empty allowlist permits nobody; `*` permits everyone.
   */
  it('classifies allowlists and permits nobody when empty', () => {
    expect(recipientAllowed('qa@example.com', [])).toBe(false)
    expect(recipientAllowed('qa@example.com', ['*@example.com'])).toBe(true)
    expect(allowlistCoverage([])).toBe('nobody')
    expect(allowlistCoverage(['qa@company.test', '*'])).toBe('everyone')
    expect(allowlistCoverage(['*@example.com'])).toBe('restricted')
  })

  /**
   * Patterns refuse spaces and separators; forward addresses also refuse glob characters.
   */
  it('validates patterns and plain addresses', () => {
    expect(isValidRecipientPattern('*@example.com')).toBe(true)
    expect(isValidRecipientPattern('a b@example.com')).toBe(false)
    expect(isValidRecipientPattern('<qa@example.com>')).toBe(false)
    expect(isValidRecipientAddress('qa+tag@example.com')).toBe(true)
    expect(isValidRecipientAddress('qa@localhost')).toBe(true)
    expect(isValidRecipientAddress('*@example.com')).toBe(false)
    expect(isValidRecipientAddress('not-an-address')).toBe(false)
  })

  /**
   * Forward recipients must be valid and allowed by the outbound's allowlist.
   */
  it('reports forward recipient problems against the allowlist', () => {
    const issues = forwardRecipientIssues(['qa@example.com', 'boss@other.test', 'broken'], ['*@example.com'])
    expect(issues).toEqual({ empty: false, tooMany: false, invalid: ['broken'], refused: ['boss@other.test'] })
    expect(hasForwardRecipientIssues(issues)).toBe(true)
    expect(hasForwardRecipientIssues(forwardRecipientIssues(['qa@example.com'], ['*@example.com']))).toBe(false)
    expect(forwardRecipientIssues([], ['*']).empty).toBe(true)
    expect(forwardRecipientIssues(Array.from({ length: 51 }, (_, index) => `qa${index}@example.com`), ['*']).tooMany).toBe(true)
  })
})
