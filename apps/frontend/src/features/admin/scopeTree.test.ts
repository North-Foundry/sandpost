import { describe, expect, it } from 'vitest'
import type { Scope } from '@/api/contracts'
import { ancestors, branchContinuations, flattenScopes, nextPosition, subtreeIds } from './scopeTree'

/**
 * Build a scope fixture containing the fields needed by the tree helpers.
 */
function scope(id: string, parentId: string | null, position = 0, name = id): Scope {
  return { id, parentId, name, description: null, filter: '', position, policyVersion: 1 }
}

describe('scope tree', () => {
  const scopes = [scope('eu', null, 1), scope('clients', null, 0), scope('acme', 'clients', 1), scope('bravo', 'clients', 0), scope('it', 'eu', 0), scope('orphan', 'missing', 0)]

  it('flattens depth-first in sibling order and keeps orphans visible', () => {
    expect(flattenScopes(scopes).map(({ scope, depth }) => `${depth}:${scope.id}`)).toEqual([
      '0:clients', '1:bravo', '1:acme', '0:orphan', '0:eu', '1:it',
    ])
  })

  it('lists ancestors from the root and stops at cycles', () => {
    expect(ancestors(scopes, 'acme').map((item) => item.id)).toEqual(['clients', 'acme'])
    expect(ancestors([scope('a', 'b'), scope('b', 'a')], 'a').map((item) => item.id)).toEqual(['b', 'a'])
  })

  it('collects subtrees and appends new siblings after the highest position', () => {
    expect([...subtreeIds(scopes, 'clients')].sort()).toEqual(['acme', 'bravo', 'clients'])
    expect(nextPosition(scopes, 'clients')).toBe(2)
    expect(nextPosition(scopes, 'clients', 'acme')).toBe(1)
    expect(nextPosition(scopes, 'it')).toBe(0)
  })

  it('reports which branches continue below each node on the path', () => {
    const byId = (id: string) => scopes.find((item) => item.id === id)!
    expect(branchContinuations(scopes, byId('bravo'))).toEqual([true, true])
    expect(branchContinuations(scopes, byId('acme'))).toEqual([true, false])
    expect(branchContinuations(scopes, byId('it'))).toEqual([false, false])
  })
})
