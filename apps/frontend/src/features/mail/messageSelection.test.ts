import { describe, expect, it } from 'vitest'
import { extendSelection, orderedSelection, selectionRange, settleSwipe, toggleSelection } from './messageSelection'

const order = ['a', 'b', 'c', 'd', 'e']

describe('message selection', () => {
  it('selects the inclusive range in list order whichever direction it is drawn', () => {
    expect(selectionRange(order, 'b', 'd')).toEqual(['b', 'c', 'd'])
    expect(selectionRange(order, 'd', 'b')).toEqual(['b', 'c', 'd'])
    expect(selectionRange(order, null, 'c')).toEqual(['c'])
    expect(selectionRange(order, 'gone', 'c')).toEqual(['c'])
    expect(selectionRange(order, 'a', 'gone')).toEqual([])
  })

  it('replaces the selection on Shift and adds to it on Cmd/Ctrl+Shift', () => {
    expect([...extendSelection(order, new Set(['e']), 'a', 'b', false)]).toEqual(['a', 'b'])
    expect([...extendSelection(order, new Set(['e']), 'a', 'b', true)].sort()).toEqual(['a', 'b', 'e'])
  })

  it('seeds a toggle with the open message and removes ids already selected', () => {
    expect([...toggleSelection(new Set(), 'c', 'a')].sort()).toEqual(['a', 'c'])
    expect([...toggleSelection(new Set(), 'c', null)]).toEqual(['c'])
    expect([...toggleSelection(new Set(['a', 'c']), 'c', 'a')]).toEqual(['a'])
  })

  it('orders the selection like the list and drops unlisted ids', () => {
    expect(orderedSelection(order, new Set(['d', 'gone', 'a']))).toEqual(['a', 'd'])
  })

  it('settles a swipe closed, on the revealed action, or as a full-swipe delete', () => {
    expect(settleSwipe(20, 400, 80)).toBe('close')
    expect(settleSwipe(60, 400, 80)).toBe('reveal')
    expect(settleSwipe(240, 400, 80)).toBe('delete')
    expect(settleSwipe(60, 0, 80)).toBe('reveal')
  })
})
