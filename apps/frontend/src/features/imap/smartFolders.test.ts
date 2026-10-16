import { describe, expect, it } from 'vitest'
import { exposableViews } from './smartFolders'

describe('exposable Smart Folders', () => {
  it('keeps sidebar folders backed by a saved View in order', () => {
    const view = { id: 'v', ownerId: null, name: 'Receipts', filter: '' }
    expect(exposableViews([
      { id: 'inbox', kind: 'inbox', name: 'Inbox', address: null, view: null },
      { id: 'a', kind: 'address', name: 'QA', address: 'qa@sandpost.local', view: null },
      { id: 'v', kind: 'smart', name: 'Receipts', address: null, view },
    ])).toEqual([view])
  })
})
