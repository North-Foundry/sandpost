import { describe, expect, it } from 'vitest'
import type { SessionUser } from '@/api/contracts'
import { createMockAdmin } from './admin'

const owner: SessionUser = {
  id: 'owner-id',
  name: 'Instance Owner',
  email: 'owner@example.test',
  role: 'owner',
}

describe('mock account visibility settings', () => {
  /**
   * Keep scoped authorization and scope membership when promoting a user to Owner.
   */
  it('preserves scoped access and assignments when a user becomes an owner', async () => {
    const admin = createMockAdmin(() => owner, [])
    const member = (await admin.users.list()).find((user) => user.id.endsWith('0002'))!
    const scope = await admin.scopes.save({
      id: 'scope-id', parentId: null, name: 'Receipts', description: null, filter: '', position: 0,
    })

    await admin.scopes.assign(scope.id, member.id)
    const promoted = await admin.users.update(member.id, { role: 'owner' })

    expect(promoted).toMatchObject({ role: 'owner', mailAccess: 'scoped' })
    expect(await admin.scopes.assignments(scope.id)).toContain(member.id)
  })

  /**
   * Include the active Owner as an ordinary mail-access user in scope assignments.
   */
  it('lists and assigns the active owner using their stored mail access', async () => {
    const admin = createMockAdmin(() => owner, [])
    const activeOwner = (await admin.users.list()).find((user) => user.id === owner.id)!
    const scope = await admin.scopes.save({
      id: 'owner-scope', parentId: null, name: 'Private', description: null, filter: '', position: 0,
    })

    expect(activeOwner.mailAccess).toBe('all')
    await admin.users.update(activeOwner.id, { mailAccess: 'scoped' })
    expect((await admin.users.list()).find((user) => user.id === owner.id)?.mailAccess).toBe('scoped')
    await admin.scopes.assign(scope.id, activeOwner.id)
    await admin.users.update(activeOwner.id, { mailAccess: 'all' })

    expect((await admin.users.list()).find((user) => user.id === owner.id)?.mailAccess).toBe('all')
    expect(await admin.scopes.assignments(scope.id)).toContain(owner.id)
  })
})
