import { describe, expect, it } from 'vitest'
import type { MessageSummary, SessionUser } from '@/api/contracts'
import { createMockPersonas, validateMockScript } from './personas'

const member: SessionUser = { id: 'member', name: 'Member', email: 'member@example.test', role: 'member' }
const viewer: SessionUser = { ...member, id: 'viewer', role: 'viewer' }
const messages: MessageSummary[] = [
  { id: 'one', sequence: 2, subject: 'Verify', from: ['app@example.test'], to: ['alice@example.test'], envelopeTo: ['Alice@example.test'], receivedAt: 1, size: 1, attachmentCount: 0, seen: false },
  { id: 'two', sequence: 1, subject: 'Other', from: ['app@example.test'], to: ['bob@example.test'], receivedAt: 1, size: 1, attachmentCount: 0, seen: false },
]

describe('mock personas', () => {
  it('manages personas with unique normalized addresses', async () => {
    let user: SessionUser | null = member
    const personas = createMockPersonas(() => user, () => messages)
    const created = await personas.create({ name: ' Alice ', address: ' ALICE@example.test', script: 'open;', enabled: true })
    expect(created).toMatchObject({ name: 'Alice', address: 'alice@example.test', revision: 1, lastExecution: null })
    await expect(personas.create({ name: 'Copy', address: 'alice@EXAMPLE.test', script: 'open;', enabled: true })).rejects.toThrow('already used by a persona')
    await expect(personas.create({ name: 'Bad', address: 'nope', script: 'open;', enabled: true })).rejects.toThrow('valid ASCII email address')
    await expect(personas.create({ name: 'Bad', address: 'c@example.test', script: 'jump;', enabled: true })).rejects.toThrow('line 1, column 1')
    expect((await personas.setEnabled(created.id, false)).enabled).toBe(false)
    expect((await personas.listMessages(created.id)).map((message) => message.id)).toEqual(['one'])
    expect(await personas.executions(created.id)).toEqual([])
    user = viewer
    await expect(personas.get(created.id)).rejects.toThrow('item not found')
    await expect(personas.create({ name: 'V', address: 'v@example.test', script: 'open;', enabled: true })).rejects.toThrow('forbidden')
    user = member
    await personas.delete(created.id)
    expect(await personas.list()).toEqual([])
  })

  it('approximates validation with positions', () => {
    expect(validateMockScript('wait 5s;\nopen;')).toMatchObject({ valid: true, statements: 2, longestWaitMilliseconds: 5000 })
    expect(validateMockScript('open;\n  jump;').error).toMatchObject({ line: 2, column: 3 })
    expect(validateMockScript('if (true) { open;').valid).toBe(false)
  })
})
