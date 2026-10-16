/**
 * Messages chosen for a bulk action, by message id. An empty selection means only the open message is active.
 */
export type MessageSelection = ReadonlySet<string>

/**
 * How a released swipe resolves: snap closed, stay open on the Delete action, or delete outright.
 */
export type SwipeOutcome = 'close' | 'reveal' | 'delete'

/**
 * A swipe past this fraction of the row's width deletes without stopping at the revealed action.
 */
export const FULL_SWIPE_RATIO = 0.6

/**
 * Returns the ids from `anchorId` to `targetId` inclusive, in list order; an unknown anchor selects the target alone.
 */
export function selectionRange(order: readonly string[], anchorId: string | null, targetId: string): string[] {
  const target = order.indexOf(targetId)
  if (target < 0) return []
  const anchor = anchorId === null ? -1 : order.indexOf(anchorId)
  if (anchor < 0) return [targetId]
  return order.slice(Math.min(anchor, target), Math.max(anchor, target) + 1)
}

/**
 * Shift-selection from the anchor to the target. A plain Shift replaces the selection with the range, while
 * Cmd/Ctrl+Shift (`additive`) adds the range to what is already selected, as in macOS list views.
 */
export function extendSelection(
  order: readonly string[],
  selection: MessageSelection,
  anchorId: string | null,
  targetId: string,
  additive: boolean,
): Set<string> {
  const range = selectionRange(order, anchorId, targetId)
  return new Set(additive ? [...selection, ...range] : range)
}

/**
 * Cmd/Ctrl-click toggling. An empty selection is first seeded with the open message, so the toggle extends
 * what the reader already sees instead of replacing it.
 */
export function toggleSelection(selection: MessageSelection, id: string, openId: string | null): Set<string> {
  const next = new Set(selection.size || openId === null ? selection : [openId])
  if (next.has(id)) next.delete(id)
  else next.add(id)
  return next
}

/**
 * The selected ids in list order, dropping ids that are no longer listed.
 */
export function orderedSelection(order: readonly string[], selection: MessageSelection): string[] {
  return order.filter((id) => selection.has(id))
}

/**
 * Decides where a released swipe settles from how far the row moved relative to its width and the action's width.
 */
export function settleSwipe(offset: number, rowWidth: number, revealWidth: number): SwipeOutcome {
  if (rowWidth > 0 && offset >= rowWidth * FULL_SWIPE_RATIO) return 'delete'
  return offset >= revealWidth / 2 ? 'reveal' : 'close'
}
