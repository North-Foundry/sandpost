import { describe, expect, it } from 'vitest'
import { formatBytes, formatListDate, mailboxName, senderInitial, sendersLabel } from './format'

describe('mail display formatting', () => {
  it('prefers quoted display names and falls back to readable local parts', () => {
    expect(mailboxName('"Ada Lovelace" <ada@example.test>')).toBe('Ada Lovelace')
    expect(mailboxName('<ada@example.test>')).toBe('<ada@example.test>')
    expect(mailboxName('build.bot@example.test')).toBe('build bot')
    expect(sendersLabel([])).toBe('Unknown sender')
    expect(sendersLabel(['a@x.test', 'Bo <b@x.test>'])).toBe('a, Bo')
  })

  it('derives an uppercase avatar initial with a placeholder for missing senders', () => {
    expect(senderInitial(['zed@example.test'])).toBe('Z')
    expect(senderInitial([])).toBe('?')
  })

  it('formats sizes and chooses date precision relative to now', () => {
    expect(formatBytes(512)).toBe('512 B')
    expect(formatBytes(2048)).toBe('2 KB')
    expect(formatBytes(3 * 1024 * 1024)).toBe('3.0 MB')
    const now = new Date(2026, 5, 15, 12)
    expect(formatListDate(new Date(2026, 5, 15, 9, 5).getTime(), now)).not.toMatch(/2026|26/)
    expect(formatListDate(new Date(2024, 0, 2).getTime(), now)).toMatch(/24/)
  })
})
