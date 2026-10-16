import { afterEach, describe, expect, it, vi } from 'vitest'
import { isSandpostDesktopBridge } from '@/api/desktop'
import { createSandpostClient, selectSandpostMode } from './client'

const bridge = {
  runtime: 'electron' as const,
  instanceStatus: vi.fn(async () => ({ initialized: true, setupKeyRequired: false as const })),
  authorizeSetup: vi.fn(async () => ({ authorization: 'opaque-setup-token' })),
  setup: vi.fn(async () => { }),
  login: vi.fn(async () => ({ id: 'owner', name: 'Owner', email: 'owner@example.test', role: 'owner' as const })),
  session: vi.fn(async () => ({ id: 'owner', name: 'Owner', email: 'owner@example.test', role: 'owner' as const })),
  logout: vi.fn(async () => { }),
  inboxes: { list: vi.fn(async () => []), create: vi.fn(async () => ({ id: 'i', name: 'Inbox', address: 'inbox@sandpost.local', system: false })) },
  views: { list: vi.fn(async () => []), save: vi.fn(async (input) => input), delete: vi.fn(async () => { }) },
  folders: { list: vi.fn(async () => []), create: vi.fn(), update: vi.fn(), delete: vi.fn(), listMessages: vi.fn(async () => []) },
  messages: {
    list: vi.fn(async () => []),
    search: vi.fn(async () => []),
    setSeen: vi.fn(async () => ({ changed: 1 })),
    copy: vi.fn(async () => ({ copied: 1 })),
    setFlags: vi.fn(async () => ({ changed: 1 })),
    get: vi.fn(async () => ({
      id: 'mail-id', facts: {
        subject: '', from: [], to: [], cc: [], received_at: 0, text: '', html: '', headers: {},
      }, attachments: []
    })),
    getRaw: vi.fn(async () => new Uint8Array()),
    getAttachment: vi.fn(async () => new Uint8Array()),
    delete: vi.fn(async () => { }),
    move: vi.fn(async () => ({ moved: 0 })),
    trash: vi.fn(async () => ({ trashed: 0 })),
  },
  trash: { listMessages: vi.fn(async () => []), empty: vi.fn(async () => ({ deleted: 0 })) },
  clipboard: { writeText: vi.fn(async () => { }) },
  events: {

    /**
 * Return a disposer for the event subscription stub.
 */
    subscribe: async () => () => { }
  },
  imap: { list: vi.fn(async () => []), folders: { create: vi.fn(), update: vi.fn(), delete: vi.fn() } },
  admin: {
    smtp: { list: vi.fn(async () => []) },
    scopes: { preview: vi.fn(async () => []) },
    users: { list: vi.fn(async () => []) },
    validateQuery: vi.fn(async () => ({ valid: true, error: null })),
  },
}

afterEach(() => {
  vi.unstubAllGlobals()
  vi.clearAllMocks()
})

describe('Electron preload bridge shape', () => {
  it('accepts the marked complete bridge surface', () => {
    expect(isSandpostDesktopBridge(bridge)).toBe(true)
  })

  it('requires the dedicated Seen function before selecting the native transport', () => {
    expect(isSandpostDesktopBridge({ ...bridge, messages: { ...bridge.messages, setSeen: undefined } })).toBe(false)
  })

  it('requires dedicated COPY and flag operations before selecting the native transport', () => {
    expect(isSandpostDesktopBridge({ ...bridge, messages: { ...bridge.messages, copy: undefined } })).toBe(false)
    expect(isSandpostDesktopBridge({ ...bridge, messages: { ...bridge.messages, setFlags: undefined } })).toBe(false)
  })

  it('rejects absent, wrong-runtime, and incomplete bridge values', () => {
    expect(isSandpostDesktopBridge(undefined)).toBe(false)
    expect(isSandpostDesktopBridge({ ...bridge, runtime: 'other' })).toBe(false)
    expect(isSandpostDesktopBridge({ ...bridge, messages: { list: bridge.messages.list } })).toBe(false)
    expect(isSandpostDesktopBridge({ ...bridge, admin: undefined })).toBe(false)
    expect(isSandpostDesktopBridge({ ...bridge, trash: undefined })).toBe(false)
    expect(isSandpostDesktopBridge({ ...bridge, trash: { listMessages: bridge.trash.listMessages } })).toBe(false)
    expect(isSandpostDesktopBridge({ ...bridge, imap: undefined })).toBe(false)
    expect(isSandpostDesktopBridge({ ...bridge, imap: { list: bridge.imap.list } })).toBe(false)
  })
})

describe('runtime selection', () => {
  it('always selects Electron when the typed bridge is present', () => {
    expect(selectSandpostMode(true, 'mock')).toBe('electron')
    expect(selectSandpostMode(true, 'http')).toBe('electron')
    expect(selectSandpostMode(true, undefined)).toBe('electron')
  })

  it('uses the Electron bridge without issuing HTTP requests', async () => {
    const fetchMock = vi.fn()
    vi.stubGlobal('fetch', fetchMock)
    vi.stubGlobal('window', { sandpost: bridge })
    const client = createSandpostClient()
    await client.messages.list({ limit: 4 })
    expect(bridge.messages.list).toHaveBeenCalledWith({ limit: 4 })
    expect(fetchMock).not.toHaveBeenCalled()
  })

  it('fails explicitly for an incomplete Electron bridge instead of falling back to HTTP', () => {
    vi.stubGlobal('window', { sandpost: { runtime: 'electron', messages: {} } })
    expect(() => createSandpostClient()).toThrow('Electron preload does not provide')
  })

  it('defaults to HTTP and opts into mock mode explicitly', () => {
    expect(selectSandpostMode(false, undefined)).toBe('http')
    expect(selectSandpostMode(false, 'http')).toBe('http')
    expect(selectSandpostMode(false, 'mock')).toBe('mock')
    expect(selectSandpostMode(false, 'development')).toBe('http')
    expect(selectSandpostMode(false, 'unexpected')).toBe('http')
  })
})
