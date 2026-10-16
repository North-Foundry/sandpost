import type { Scope } from '@/api/contracts'

/**
 * One scope placed in depth-first tree order.
 */
export interface ScopeRow {
  scope: Scope
  depth: number
}

/**
 * Order siblings by their saved position, then by name for stable ties.
 */
function bySiblingOrder(first: Scope, second: Scope): number {
  return first.position - second.position || first.name.localeCompare(second.name)
}

/**
 * Return the children of a parent (or the roots for `null`) in sibling order.
 */
export function siblings(scopes: Scope[], parentId: string | null): Scope[] {
  return scopes.filter((scope) => scope.parentId === parentId).sort(bySiblingOrder)
}

/**
 * Flatten the hierarchy depth-first. Scopes whose parent is missing are shown as roots so a
 * partially loaded or inconsistent tree never hides a definition.
 */
export function flattenScopes(scopes: Scope[]): ScopeRow[] {
  const known = new Set(scopes.map((scope) => scope.id))
  const rows: ScopeRow[] = []
  const visited = new Set<string>()

  /**
   * Append a scope and its ordered descendants once, guarding against cyclic input.
   */
  const visit = (scope: Scope, depth: number) => {
    if (visited.has(scope.id)) return
    visited.add(scope.id)
    rows.push({ scope, depth })
    for (const child of siblings(scopes, scope.id)) visit(child, depth + 1)
  }
  for (const root of scopes.filter((scope) => !scope.parentId || !known.has(scope.parentId)).sort(bySiblingOrder)) visit(root, 0)
  return rows
}

/**
 * Return the ancestors of `parentId`, from the root down to the parent itself.
 */
export function ancestors(scopes: Scope[], parentId: string | null): Scope[] {
  const chain: Scope[] = []
  const seen = new Set<string>()
  for (let current = parentId; current && !seen.has(current);) {
    seen.add(current)
    const scope = scopes.find((candidate) => candidate.id === current)
    if (!scope) break
    chain.unshift(scope)
    current = scope.parentId
  }
  return chain
}

/**
 * Return a scope's identifier together with every descendant identifier.
 */
export function subtreeIds(scopes: Scope[], id: string): Set<string> {
  const result = new Set([id])
  let grew = true
  while (grew) {
    grew = false
    for (const scope of scopes) {
      if (scope.parentId && result.has(scope.parentId) && !result.has(scope.id)) {
        result.add(scope.id)
        grew = true
      }
    }
  }
  return result
}

/**
 * Return the position that places a scope after its future siblings.
 */
export function nextPosition(scopes: Scope[], parentId: string | null, excludeId?: string): number {
  return siblings(scopes, parentId)
    .filter((scope) => scope.id !== excludeId)
    .reduce((highest, scope) => Math.max(highest, scope.position + 1), 0)
}
