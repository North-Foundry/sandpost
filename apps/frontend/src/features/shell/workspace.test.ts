import { afterEach, describe, expect, it, vi } from 'vitest'
import type { Inbox, MessageSummary, SandpostClient, SavedView, SessionUser } from '@/api/contracts'
import { createWorkspace, DEFAULT_INBOX, TRASH_FOLDER_ID } from './workspace'

const owner: SessionUser = { id: 'owner', name: 'Owner', email: 'owner@example.test', role: 'owner' }
const member: SessionUser = { ...owner, id: 'member', role: 'member' }
const addressInbox: Inbox = { id: 'address', name: 'QA', address: 'qa@sandpost.local', system: false }
const addressView: SavedView = { id: 'address', ownerId: 'owner', name: 'QA', filter: 'envelope.to.address == "qa@sandpost.local"' }
const smartView: SavedView = { id: 'smart', ownerId: 'owner', name: 'Receipts', filter: 'subject contains "Receipt"' }

/**
 * Create a promise whose resolution the test controls to model a pending request.
 */
function deferred<T>() {
  let resolve!: (value: T) => void
  const promise = new Promise<T>((done) => { resolve = done })
  return { promise, resolve }
}

/**
 * Drain the promise continuations used by workspace startup and refresh flows.
 */
async function flush(): Promise<void> {
  for (let index = 0; index < 6; index += 1) await Promise.resolve()
}

/**
 * Build a client double with session, inbox, view, event, and message responses.
 */
function createClient(user: SessionUser | null): SandpostClient {
  return {
    session: vi.fn(async () => user),
    logout: vi.fn(async () => { }),
    inboxes: { list: vi.fn(async () => [DEFAULT_INBOX, addressInbox]), create: vi.fn() },
    views: { list: vi.fn(async () => [addressView, smartView]), save: vi.fn(async (view) => view), delete: vi.fn(async () => { }) },
    folders: { list: vi.fn(async () => []), create: vi.fn(), update: vi.fn(), delete: vi.fn(), listMessages: vi.fn(async () => []) },
    imap: { list: vi.fn(async () => []) },
    events: { subscribe: vi.fn(async () => () => { }) },
    messages: { list: vi.fn(async () => []), setSeen: vi.fn(async () => 1) },
  } as unknown as SandpostClient
}

describe('workspace', () => {
  let stop: (() => void) | undefined
  afterEach(() => { stop?.() })

  it('lists Trash after the inbox, then address inboxes and smart folders once each', async () => {
    const created = createWorkspace(createClient(owner), { onSignedOut: vi.fn() })
    stop = created.stop
    created.start()
    await flush()
    expect(created.workspace.folders.value.map((folder) => [folder.kind, folder.id])).toEqual([
      ['inbox', DEFAULT_INBOX.id], ['trash', TRASH_FOLDER_ID], ['address', 'address'], ['smart', 'smart'],
    ])
    expect(created.workspace.findFolder(undefined)?.kind).toBe('inbox')
    expect(created.workspace.findFolder('trash')).toMatchObject({ kind: 'trash', name: 'Trash', view: null, manual: null })
    expect(created.workspace.findFolder('address')?.view).toEqual(addressView)
  })

  it('keeps a created inbox when an older folder-list request resolves afterward', async () => {
    const client = createClient(owner)
    const staleList = deferred<Inbox[]>()
    vi.mocked(client.inboxes.list).mockReturnValueOnce(staleList.promise)
    const created: Inbox = { id: 'created', name: 'Signup', address: 'signup@sandpost.local', system: false }
    vi.mocked(client.inboxes.create).mockResolvedValueOnce(created)
    const { workspace, start, stop: stopWorkspace } = createWorkspace(client, { onSignedOut: vi.fn() })
    stop = stopWorkspace
    start()

    await workspace.createAddressInbox({ name: 'Signup', address: 'signup@sandpost.local' })
    staleList.resolve([DEFAULT_INBOX])
    await flush()
    expect(workspace.findFolder('created')).toMatchObject({ kind: 'address', address: 'signup@sandpost.local' })
  })

  it('shows administration to owners and admins only', async () => {
    for (const [user, expected] of [
      [owner, true], [{ ...owner, role: 'admin' }, true], [member, false], [{ ...member, role: 'viewer' }, false],
    ] as const) {
      const created = createWorkspace(createClient(user as SessionUser), { onSignedOut: vi.fn() })
      created.start()
      await flush()
      expect(created.workspace.canAdminister.value).toBe(expected)
      expect(created.workspace.capabilitiesLoaded.value).toBe(true)
      created.stop()
    }
  })

  it('updates address inbox names and icons immediately while preserving their recipient filter', async () => {
    const client = createClient(owner)
    const created = createWorkspace(client, { onSignedOut: vi.fn() })
    stop = created.stop
    created.start()
    await flush()
    const saved = { ...addressView, name: 'Testing', icon: 'flask-conical' }
    await created.workspace.saveView(saved)
    expect(client.views.save).toHaveBeenCalledWith(saved)
    expect(created.workspace.findFolder('address')).toMatchObject({
      kind: 'address', name: 'Testing', address: addressInbox.address,
      view: { icon: 'flask-conical', filter: addressView.filter },
    })
    await created.workspace.saveView({ ...saved, icon: null })
    expect(created.workspace.findFolder('address')?.view?.icon).toBeNull()
  })

  it('returns to login when the session is missing and after signing out', async () => {
    const onSignedOut = vi.fn()
    const signedOut = createWorkspace(createClient(null), { onSignedOut })
    signedOut.start()
    await flush()
    expect(onSignedOut).toHaveBeenCalledOnce()
    signedOut.stop()

    const client = createClient(owner)
    const created = createWorkspace(client, { onSignedOut })
    stop = created.stop
    await created.workspace.logout()
    expect(client.logout).toHaveBeenCalledOnce()
    expect(onSignedOut).toHaveBeenCalledTimes(2)
  })

  it('removes a deleted folder locally and forwards events to listeners until stopped', async () => {
    const client = createClient(owner)
    let dispatch!: Parameters<SandpostClient['events']['subscribe']>[0]
    vi.mocked(client.events.subscribe).mockImplementation(async (listener) => { dispatch = listener; return () => { } })
    const created = createWorkspace(client, { onSignedOut: vi.fn() })
    created.start()
    await flush()

    await created.workspace.deleteView('smart')
    expect(created.workspace.findFolder('smart')).toBeNull()

    const received: string[] = []
    created.workspace.onMailEvent((event) => received.push(event.type))
    dispatch({ type: 'disconnected' })
    expect(created.workspace.connection.value).toBe('disconnected')
    dispatch({ type: 'ready' })
    expect(created.workspace.connection.value).toBe('connected')
    created.stop()
    dispatch({ type: 'resync' })
    expect(received).toEqual(['disconnected', 'ready'])
  })

  it('counts unseen Inbox messages across pages, recounts after mail events, and updates when one is marked read', async () => {
    vi.useFakeTimers()
    try {
      const client = createClient(owner)

      /**
 * Build a page of unread message summaries with descending sequence numbers.
 */
      const summaries = (count: number, offset = 0) => Array.from({ length: count }, (_, index) => ({ id: `m${offset + index}`, sequence: 1000 - offset - index, seen: false }) as MessageSummary)
      vi.mocked(client.messages.list).mockResolvedValueOnce(summaries(100)).mockResolvedValueOnce(summaries(20, 100))
      let dispatch!: Parameters<SandpostClient['events']['subscribe']>[0]
      vi.mocked(client.events.subscribe).mockImplementation(async (listener) => { dispatch = listener; return () => { } })
      const created = createWorkspace(client, { onSignedOut: vi.fn() })
      stop = created.stop
      expect(created.workspace.inboxUnreadCount.value).toBeNull()
      created.start()
      await flush()
      expect(vi.mocked(client.messages.list).mock.calls.map(([options]) => options)).toEqual([{ before: undefined, limit: 100 }, { before: 901, limit: 100 }])
      expect(created.workspace.inboxUnreadCount.value).toBe(120)

      await created.workspace.setSeen(['m0'], true)
      await created.workspace.setSeen(['m0'], true)
      expect(created.workspace.inboxUnreadCount.value).toBe(119)

      vi.mocked(client.messages.list).mockResolvedValueOnce(summaries(3).map((message) => ({ ...message, seen: message.id === 'm0' })))
      dispatch({ type: 'message', id: 'm0', sequence: 1000 })
      dispatch({ type: 'message', id: 'm1', sequence: 999 })
      await vi.runAllTimersAsync()
      expect(client.messages.list).toHaveBeenCalledTimes(3)
      expect(created.workspace.inboxUnreadCount.value).toBe(2)
    } finally { vi.useRealTimers() }
  })

  it('rejects a stale unread scan after an accepted shared Seen write', async () => {
    vi.useFakeTimers()
    try {
      const client = createClient(owner)
      const oldPage = deferred<MessageSummary[]>()
      vi.mocked(client.messages.list).mockReturnValueOnce(oldPage.promise)
        .mockResolvedValue([{ id: 'm0', sequence: 1, seen: true } as MessageSummary])
      const created = createWorkspace(client, { onSignedOut: vi.fn() })
      stop = created.stop
      created.start()
      await flush()
      await created.workspace.setSeen(['m0'], true)
      oldPage.resolve([{ id: 'm0', sequence: 1, seen: false } as MessageSummary])
      await flush()
      expect(created.workspace.inboxUnreadCount.value).toBeNull()
      await vi.runAllTimersAsync()
      expect(created.workspace.inboxUnreadCount.value).toBe(0)
      expect(created.workspace.viewedMessageIds.value.has('m0')).toBe(true)
    } finally { vi.useRealTimers() }
  })

  it('keeps counting past server Seen mail until the unread limit is reached, and rescans when a Seen change frees room', async () => {
    vi.useFakeTimers()
    const seenIdentifiers = new Set(Array.from({ length: 10 }, (_, index) => `m${index}`))
    try {
      const client = createClient(owner)
      vi.mocked(client.messages.setSeen).mockImplementation(async (identifiers, seen) => {
        for (const id of identifiers) seen ? seenIdentifiers.add(id) : seenIdentifiers.delete(id)
        return identifiers.length
      })
      /*
       * 1,500 messages, newest first, with sequences 1500 down to 1.
       */
      vi.mocked(client.messages.list).mockImplementation(async (options) => {
        const newest = Math.min(options?.before !== undefined ? options.before - 1 : 1500, 1500)
        const count = Math.max(0, Math.min(options?.limit ?? 50, newest))
        return Array.from({ length: count }, (_, index) => ({ id: `m${1500 - newest + index}`, sequence: newest - index, seen: seenIdentifiers.has(`m${1500 - newest + index}`) }) as MessageSummary)
      })
      const created = createWorkspace(client, { onSignedOut: vi.fn() })
      stop = created.stop
      created.start()
      await vi.runAllTimersAsync()
      /*
       * Ten pages hold only 990 unread messages, so the scan reads an eleventh page and stops there.
       */
      expect(client.messages.list).toHaveBeenCalledTimes(11)
      expect(created.workspace.inboxUnreadCount.value).toBe(1090)

      await created.workspace.setSeen(['m20'], true)
      await vi.runAllTimersAsync()
      expect(client.messages.list).toHaveBeenCalledTimes(22)
      expect(created.workspace.inboxUnreadCount.value).toBe(1089)
    } finally {
      vi.useRealTimers()
      vi.unstubAllGlobals()
    }
  })
})
