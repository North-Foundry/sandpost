import { afterEach, describe, expect, it, vi } from 'vitest'
import { createHttpClient } from './http'

describe('HTTP adapter', () => {
  afterEach(() => vi.unstubAllGlobals())

  it('sends the remember-me choice and restores a session through cookies in a fresh client', async () => {
    const user = { id: 'owner-id', name: 'Owner', email: 'owner@example.test', role: 'owner' }
    const fetchMock = vi.fn()
      .mockResolvedValueOnce({
        ok: true,

        /**
         * Return the successful login response for the HTTP adapter.
         */
        json: async () => ({ token: 'memory-token', user }),
      })
      .mockResolvedValueOnce({
        ok: true,

        /**
         * Return the session associated with the browser cookie.
         */
        json: async () => user,
      })
    vi.stubGlobal('fetch', fetchMock)

    const loginClient = createHttpClient()
    await loginClient.login({ email: user.email, password: 'password', rememberMe: true })
    const freshClient = createHttpClient()
    expect(await freshClient.session()).toEqual(user)

    expect(JSON.parse(String(fetchMock.mock.calls[0]![1]?.body))).toEqual({
      email: user.email,
      password: 'password',
      rememberMe: true,
    })
    expect(fetchMock.mock.calls[1]![1]).toMatchObject({ credentials: 'same-origin' })
    expect(new Headers(fetchMock.mock.calls[1]![1]?.headers).has('Authorization')).toBe(false)
  })

  it('keeps the bearer token in memory and uses the session cookie for SSE', async () => {
    const user = { id: 'owner-id', name: 'Owner', email: 'owner@example.test', role: 'owner' }
    const fetchMock = vi.fn()
      .mockResolvedValueOnce({
        ok: true,

        /**
         * Return the JSON response payload configured for this adapter test.
         */
        json: async () => ({ initialized: false, setupKeyRequired: true })
      })
      .mockResolvedValueOnce({
        ok: true, status: 201,

        /**
         * Return the JSON response payload configured for this adapter test.
         */
        json: async () => ({ initialized: true, setupKeyRequired: false })
      })
      .mockResolvedValueOnce({
        ok: true,

        /**
         * Return the JSON response payload configured for this adapter test.
         */
        json: async () => ({ token: 'memory-token', user })
      })
      .mockResolvedValueOnce({
        ok: true,

        /**
         * Return the JSON response payload configured for this adapter test.
         */
        json: async () => user
      })
      .mockResolvedValueOnce({ ok: true, status: 204 })
      .mockResolvedValueOnce({
        ok: true,

        /**
         * Return the JSON response payload configured for this adapter test.
         */
        json: async () => user
      })
    vi.stubGlobal('fetch', fetchMock)
    const client = createHttpClient()
    expect(await client.instanceStatus()).toEqual({ initialized: false, setupKeyRequired: true })
    await client.setup({ bootstrapSecret: 'secret', name: 'Owner', email: user.email, password: 'password' })
    expect(await client.login({ email: user.email, password: 'password' })).toEqual(user)
    expect(await client.session()).toEqual(user)
    await client.logout()
    expect(await client.session()).toEqual(user)

    expect(fetchMock.mock.calls.map(([path]) => path)).toEqual([
      '/api/v1/instance', '/api/v1/setup', '/api/v1/login', '/api/v1/session', '/api/v1/logout', '/api/v1/session',
    ])
    expect(fetchMock.mock.calls[3]![1]).toMatchObject({ credentials: 'same-origin' })
    expect(new Headers(fetchMock.mock.calls[3]![1]?.headers).get('Authorization')).toBe('Bearer memory-token')
    expect(fetchMock.mock.calls[4]![1]).toMatchObject({ method: 'POST', credentials: 'same-origin' })
    expect(new Headers(fetchMock.mock.calls[4]![1]?.headers).get('Authorization')).toBe('Bearer memory-token')
    expect(fetchMock.mock.calls[5]![1]).toMatchObject({ credentials: 'same-origin' })
    expect(new Headers(fetchMock.mock.calls[5]![1]?.headers).has('Authorization')).toBe(false)

    const source = { addEventListener: vi.fn(), removeEventListener: vi.fn(), close: vi.fn(), onerror: null }
    vi.stubGlobal('EventSource', vi.fn(function MockEventSource(_url: string, options: EventSourceInit) {
      expect(options.withCredentials).toBe(true)
      return source
    }))
    const stop = await client.events.subscribe(() => { })
    stop()
  })

  it('reads and writes saved views and passes view and DSL filters to the message list', async () => {
    const wireView = { identifier: 'view-id', owner_identifier: 'user-id', name: 'Receipts', filter: 'subject contains "x"', icon: 'receipt' }
    const fetchMock = vi.fn()
      .mockResolvedValueOnce({
        ok: true,

        /**
         * Return the JSON response payload configured for this adapter test.
         */
        json: async () => [wireView]
      })
      .mockResolvedValueOnce({
        ok: true,

        /**
         * Return the JSON response payload configured for this adapter test.
         */
        json: async () => wireView
      })
      .mockResolvedValueOnce({ ok: true, status: 204 })
      .mockResolvedValueOnce({
        ok: true,

        /**
         * Return the JSON response payload configured for this adapter test.
         */
        json: async () => []
      })
    vi.stubGlobal('fetch', fetchMock)
    const client = createHttpClient()
    const view = { id: 'view-id', ownerId: 'user-id', name: 'Receipts', filter: 'subject contains "x"', icon: 'receipt' }
    expect(await client.views.list()).toEqual([view])
    expect(await client.views.save(view)).toEqual(view)
    await client.views.delete('view-id')
    await client.messages.list({ viewId: 'view-id', query: 'size > 1', limit: 5 })

    expect(fetchMock.mock.calls.map(([path, init]) => [path, init?.method ?? 'GET'])).toEqual([
      ['/api/v1/views', 'GET'], ['/api/v1/views/view-id', 'PUT'], ['/api/v1/views/view-id', 'DELETE'],
      ['/api/v1/messages?limit=5&view_id=view-id&q=size+%3E+1', 'GET'],
    ])
    expect(JSON.parse(String(fetchMock.mock.calls[1]![1]?.body))).toEqual(wireView)
  })

  it('posts the setup key for temporary authorization and preserves public error codes', async () => {
    const fetchMock = vi.fn()
      .mockResolvedValueOnce({
        ok: true,

        /**
         * Return the JSON response payload configured for this adapter test.
         */
        json: async () => ({ initialized: false, setupKeyRequired: true })
      })
      .mockResolvedValueOnce({
        ok: true, status: 200,

        /**
         * Return the JSON response payload configured for this adapter test.
         */
        json: async () => ({ authorization: 'opaque-authorization' })
      })
      .mockResolvedValueOnce({
        ok: false, status: 401,

        /**
         * Expose a response clone so the HTTP adapter can read the mocked error payload.
         */
        clone: () => ({
          /**
           * Return the JSON response payload configured for this adapter test.
           */
          json: async () => ({ error: 'Invalid setup key.', code: 'authentication_failed' })
        }),
      })
    vi.stubGlobal('fetch', fetchMock)
    const client = createHttpClient()
    expect(await client.instanceStatus()).toEqual({ initialized: false, setupKeyRequired: true })
    expect(await client.authorizeSetup({ setupKey: 'startup-key' })).toEqual({ authorization: 'opaque-authorization' })
    expect(fetchMock).toHaveBeenNthCalledWith(2, '/api/v1/setup/authorize', expect.objectContaining({
      method: 'POST', credentials: 'same-origin', body: JSON.stringify({ setupKey: 'startup-key' }),
    }))
    expect(new Headers(fetchMock.mock.calls[1]![1]?.headers).get('Content-Type')).toBe('application/json')
    await expect(client.authorizeSetup({ setupKey: 'bad-key' })).rejects.toMatchObject({
      status: 401, code: 'authentication_failed',
      message: 'Sandpost API 401 [authentication_failed]: Invalid setup key.: /api/v1/setup/authorize',
    })
  })

  it('clears its in-memory token after a failed logout request', async () => {
    const fetchMock = vi.fn()
      .mockResolvedValueOnce({
        ok: true,

        /**
         * Return the JSON response payload configured for this adapter test.
         */
        json: async () => ({ token: 'memory-token', user: {} })
      })
      .mockResolvedValueOnce({
        ok: false, status: 503,

        /**
         * Expose a response clone so the HTTP adapter can read the mocked error payload.
         */
        clone: () => ({
          /**
           * Return the JSON response payload configured for this adapter test.
           */
          json: async () => ({ error: 'unavailable' })
        }),
      })
      .mockResolvedValueOnce({
        ok: true,

        /**
         * Return the JSON response payload configured for this adapter test.
         */
        json: async () => ({ initialized: true })
      })
    vi.stubGlobal('fetch', fetchMock)
    const client = createHttpClient()
    await client.login({ email: 'owner@example.test', password: 'password' })
    await expect(client.logout()).rejects.toThrow('unavailable')
    await client.instanceStatus()
    expect(new Headers(fetchMock.mock.calls[1]![1]?.headers).get('Authorization')).toBe('Bearer memory-token')
    expect(new Headers(fetchMock.mock.calls[2]![1]?.headers).has('Authorization')).toBe(false)
  })

  it('builds keyset query parameters and normalizes summary, detail, and raw responses', async () => {
    const fetchMock = vi.fn()
      .mockResolvedValueOnce({
        ok: true,

        /**
         * Return the JSON response payload configured for this adapter test.
         */
        json: async () => [{
          id: 'mail-id', seq: 19, subject: 'Hello', from: [{ address: 'sender@example.test', domain: 'example.test' }],
          to: [], received_at: 42, size: 100, attachment_count: 1,
        }]
      })
      .mockResolvedValueOnce({
        ok: true,

        /**
         * Return the JSON response payload configured for this adapter test.
         */
        json: async () => ({
          id: 'mail-id', facts: {
            subject: 'Hello', from: [{ address: 'sender@example.test', domain: 'example.test' }], to: [], cc: [],
            received_at: 42, text: 'body', html: '', headers: { subject: ['Hello'] },
          },
          attachments: [{ filename: 'a.txt', content_type: 'text/plain', size: 3, content_hash: 'hash' }],
          html_generated: true,
        })
      })
      .mockResolvedValueOnce({
        ok: true,

        /**
         * Return the binary response bytes supplied by this HTTP test fixture.
         */
        arrayBuffer: async () => new Uint8Array([0, 255, 42]).buffer
      })
      .mockResolvedValueOnce({
        ok: true,

        /**
         * Return the compatibility report payload configured for this adapter test.
         */
        json: async () => ({
          html_present: true, external_stylesheets: 1, element_count: 4,
          data: { source: 'caniemail.com', url: 'https://www.caniemail.com', updated: '2026-09-16', license: 'MIT' },
          clients: [{ id: 'gmail.android', family: 'gmail', family_name: 'Gmail', platform: 'android', platform_name: 'Android', main: true }],
          summary: { clients: 1, identical: 0, with_differences: 0, with_problems: 1, untested: 0 },
        weighted: { supported: 50, partial: 0, unsupported: 50 },
          features: [{
            slug: 'css-display-grid', title: 'display:grid', category: 'css', url: 'https://www.caniemail.com/features/css-display-grid/',
            description: 'Grid layout.', notes: { '1': 'Note.' }, occurrences: 2, examples: ['display: grid'], selector_unverified: false, scored: true, graceful: false, impact: 50,
            support: [{ client: 'gmail.android', version: '2026-03', latest: true, support: 'no' as const, notes: [1] }],
          }],
        })
      })
    vi.stubGlobal('fetch', fetchMock)
    const client = createHttpClient()

    expect(await client.messages.list({ before: 20, limit: 10 })).toMatchObject([{
      id: 'mail-id', sequence: 19, from: ['sender@example.test'], receivedAt: 42000, attachmentCount: 1,
    }])
    expect(fetchMock).toHaveBeenNthCalledWith(1, '/api/v1/messages?before=20&limit=10', expect.objectContaining({ credentials: 'same-origin' }))
    expect(await client.messages.get('mail/id')).toMatchObject({
      id: 'mail-id', subject: 'Hello', text: 'body', receivedAt: 42000,
      attachments: [{ filename: 'a.txt', contentType: 'text/plain', size: 3 }], htmlGenerated: true,
    })
    expect(fetchMock).toHaveBeenNthCalledWith(2, '/api/v1/messages/mail%2Fid', expect.objectContaining({ credentials: 'same-origin' }))
    expect(await client.messages.getRaw('mail/id')).toEqual(new Uint8Array([0, 255, 42]))
    expect(fetchMock).toHaveBeenNthCalledWith(3, '/api/v1/messages/mail%2Fid/raw', expect.objectContaining({ credentials: 'same-origin' }))
    expect(await client.messages.htmlCheck('mail/id')).toMatchObject({
      htmlPresent: true, externalStylesheets: 1, elementCount: 4, weighted: { supported: 50, unsupported: 50 },
      clients: [{ id: 'gmail.android', familyName: 'Gmail', platformName: 'Android', main: true }],
      summary: { clients: 1, withProblems: 1 },
      features: [{
        slug: 'css-display-grid', occurrences: 2, examples: ['display: grid'], selectorUnverified: false, scored: true, impact: 50, notes: { '1': 'Note.' },
        support: [{ client: 'gmail.android', version: '2026-03', latest: true, support: 'no', notes: [1] }],
      }],
    })
    expect(fetchMock).toHaveBeenNthCalledWith(4, '/api/v1/messages/mail%2Fid/html-check', expect.objectContaining({ credentials: 'same-origin' }))
  })

  it('reads, starts and bulk-starts shared link checks with snake-case wire fields', async () => {
    const report = {
      message_id: 'mail/id', running: true, started_at: 1_700_000_100, checked_at: 1_700_000_000, urls: ['https://example.test/', 'http://localhost:3000/'], omitted_links: 2,
      summary: { total: 2, healthy: 1, broken: 0, server_error: 0, failed: 0, inconclusive: 1 },
      links: [
        { url: 'https://example.test/', outcome: 'healthy', status_code: 200, status_text: 'OK', final_url: 'https://www.example.test/', redirects: 1, error_kind: null, error_message: null, checked_at: 1_700_000_000 },
        { url: 'http://localhost:3000/', outcome: 'inconclusive', status_code: null, status_text: null, final_url: null, redirects: 0, error_kind: 'blocked_address', error_message: '127.0.0.1 is not a public address', checked_at: 1_700_000_000 },
      ],
    }
    const fetchMock = vi.fn()
      .mockResolvedValueOnce({ ok: true, json: async () => report })
      .mockResolvedValueOnce({ ok: true, json: async () => ({ ...report, running: true }) })
      .mockResolvedValueOnce({ ok: true, json: async () => ({ started: 1, already_running: 0, skipped: 1 }) })
      .mockResolvedValueOnce({ ok: true, json: async () => ({ id: 'member', name: 'Member', email: 'm@example.test', role: 'member' }) })
    vi.stubGlobal('fetch', fetchMock)
    const client = createHttpClient()

    expect(await client.messages.linkCheck('mail/id')).toEqual({
      messageId: 'mail/id', running: true, startedAt: 1_700_000_100, checkedAt: 1_700_000_000, urls: ['https://example.test/', 'http://localhost:3000/'], omittedLinks: 2,
      summary: { total: 2, healthy: 1, broken: 0, serverError: 0, failed: 0, inconclusive: 1 },
      links: [
        { url: 'https://example.test/', outcome: 'healthy', statusCode: 200, statusText: 'OK', finalUrl: 'https://www.example.test/', redirects: 1, errorKind: null, errorMessage: null, checkedAt: 1_700_000_000 },
        { url: 'http://localhost:3000/', outcome: 'inconclusive', statusCode: null, statusText: null, finalUrl: null, redirects: 0, errorKind: 'blocked_address', errorMessage: '127.0.0.1 is not a public address', checkedAt: 1_700_000_000 },
      ],
    })
    expect(fetchMock).toHaveBeenNthCalledWith(1, '/api/v1/messages/mail%2Fid/link-check', expect.objectContaining({ method: 'GET' }))
    expect(await client.messages.startLinkCheck('mail/id')).toMatchObject({ running: true })
    expect(fetchMock).toHaveBeenNthCalledWith(2, '/api/v1/messages/mail%2Fid/link-check', expect.objectContaining({ method: 'POST' }))
    expect(await client.messages.checkLinks(['a', 'b'])).toEqual({ started: 1, alreadyRunning: 0, skipped: 1 })
    expect(fetchMock).toHaveBeenNthCalledWith(3, '/api/v1/messages/link-check', expect.objectContaining({ method: 'POST', body: JSON.stringify({ identifiers: ['a', 'b'] }) }))
    expect(await client.session()).toEqual({ id: 'member', name: 'Member', email: 'm@example.test', role: 'member' })
  })

  it('reports HTTP status errors for list, detail, and raw requests', async () => {
    const fetchMock = vi.fn().mockResolvedValue({
      ok: false, status: 404,

      /**
       * Expose a response clone so the HTTP adapter can read the mocked error payload.
       */
      clone: () => ({
        /**
         * Return the JSON response payload configured for this adapter test.
         */
        json: async () => ({ error: 'message not found' })
      }),
    })
    vi.stubGlobal('fetch', fetchMock)
    const client = createHttpClient()
    await expect(client.messages.list()).rejects.toThrow('Sandpost API 404: message not found: /api/v1/messages')
    await expect(client.messages.get('missing')).rejects.toThrow('Sandpost API 404: message not found: /api/v1/messages/missing')
    await expect(client.messages.getRaw('missing')).rejects.toThrow('Sandpost API 404: message not found: /api/v1/messages/missing/raw')
  })

  it('uses the inbox, search, deletion, and binary attachment HTTP contracts', async () => {
    const inboxes = [{ id: 'inbox-id', name: 'Default Inbox', address: null, system: true }]
    const summary = { id: 'mail-id', seq: 4, subject: 'Hello', from: [], to: [], received_at: 3, size: 9, attachment_count: 0, preview: 'Body' }
    const bytes = new Uint8Array([0, 255, 17])
    const fetchMock = vi.fn()
      .mockResolvedValueOnce({
        ok: true,

        /**
         * Return the JSON response payload configured for this adapter test.
         */
        json: async () => inboxes
      })
      .mockResolvedValueOnce({
        ok: true,

        /**
         * Return the JSON response payload configured for this adapter test.
         */
        json: async () => ({ id: 'new-id', name: 'QA', address: 'qa+tag@dev', system: false })
      })
      .mockResolvedValueOnce({
        ok: true,

        /**
         * Return the JSON response payload configured for this adapter test.
         */
        json: async () => [summary]
      })
      .mockResolvedValueOnce({
        ok: true,

        /**
         * Return the JSON response payload configured for this adapter test.
         */
        json: async () => [summary]
      })
      .mockResolvedValueOnce({ ok: true, status: 204 })
      .mockResolvedValueOnce({
        ok: true,

        /**
         * Return the binary response bytes supplied by this HTTP test fixture.
         */
        arrayBuffer: async () => bytes.buffer
      })
    vi.stubGlobal('fetch', fetchMock)
    const client = createHttpClient()

    expect(await client.inboxes.list()).toEqual(inboxes)
    expect(await client.inboxes.create({ name: ' QA ', address: ' QA+Tag@Dev ' })).toMatchObject({ address: 'qa+tag@dev' })
    expect(fetchMock).toHaveBeenNthCalledWith(2, '/api/v1/inboxes', {
      method: 'POST', headers: expect.any(Headers), credentials: 'same-origin',
      body: JSON.stringify({ name: 'QA', address: 'qa+tag@dev' }),
    })
    expect(new Headers(fetchMock.mock.calls[1]![1]?.headers).get('Content-Type')).toBe('application/json')
    expect(await client.messages.list({ before: 5, limit: 2, inboxId: 'inbox-id' })).toMatchObject([{ preview: 'Body' }])
    expect(fetchMock).toHaveBeenNthCalledWith(3, '/api/v1/messages?before=5&limit=2&inbox_id=inbox-id', expect.objectContaining({ credentials: 'same-origin' }))
    expect(await client.messages.search('a+b', { before: 6, limit: 3 })).toHaveLength(1)
    expect(fetchMock).toHaveBeenNthCalledWith(4, '/api/v1/search?q=content+contains+%22a%2Bb%22&before=6&limit=3', expect.objectContaining({ credentials: 'same-origin' }))
    await expect(client.messages.delete('mail-id')).resolves.toBeUndefined()
    expect(fetchMock).toHaveBeenNthCalledWith(5, '/api/v1/messages/mail-id', expect.objectContaining({ method: 'DELETE', credentials: 'same-origin' }))
    expect(await client.messages.getAttachment('mail-id', 2)).toEqual(bytes)
    expect(fetchMock).toHaveBeenNthCalledWith(6, '/api/v1/messages/mail-id/attachments/2', expect.objectContaining({ credentials: 'same-origin' }))
  })

  it('uses manual folder CRUD, scoped message listing, and placement HTTP contracts', async () => {
    const folder = {
      identifier: 'folder-id', owner_identifier: 'user-id', name: 'Review', exposed_accounts: ['imap-id'],
      created_at: 10, updated_at: 11,
    }
    const fetchMock = vi.fn()
      .mockResolvedValueOnce({
        ok: true,

        /**
         * Return the JSON response payload configured for this adapter test.
         */
        json: async () => [folder]
      })
      .mockResolvedValueOnce({
        ok: true,

        /**
         * Return the JSON response payload configured for this adapter test.
         */
        json: async () => folder
      })
      .mockResolvedValueOnce({
        ok: true,

        /**
         * Return the JSON response payload configured for this adapter test.
         */
        json: async () => folder
      })
      .mockResolvedValueOnce({
        ok: true,

        /**
         * Return the JSON response payload configured for this adapter test.
         */
        json: async () => []
      })
      .mockResolvedValueOnce({
        ok: true,

        /**
         * Return the JSON response payload configured for this adapter test.
         */
        json: async () => ({ moved: 2 })
      })
      .mockResolvedValueOnce({ ok: true, status: 204 })
    vi.stubGlobal('fetch', fetchMock)
    const client = createHttpClient()
    expect(await client.folders.list()).toMatchObject([{ id: 'folder-id', ownerId: 'user-id', exposedAccounts: ['imap-id'], createdAt: 10_000 }])
    await client.folders.create({ name: 'Review' })
    expect(fetchMock.mock.calls[1]?.[1]).toMatchObject({ method: 'POST', body: JSON.stringify({ name: 'Review' }) })
    await client.folders.update('folder/id', { name: 'Review' })
    expect(fetchMock.mock.calls[2]?.[0]).toBe('/api/v1/folders/folder%2Fid')
    await client.folders.listMessages('folder/id', { before: 9, limit: 4, query: 'subject == "x"' })
    expect(fetchMock.mock.calls[3]?.[0]).toBe('/api/v1/folders/folder%2Fid/messages?before=9&limit=4&q=subject+%3D%3D+%22x%22')
    expect(await client.messages.move(['one', 'two'], 'folder-id')).toBe(2)
    expect(fetchMock.mock.calls[4]?.[1]).toMatchObject({ method: 'POST', body: JSON.stringify({ identifiers: ['one', 'two'], folderIdentifier: 'folder-id' }) })
    await client.folders.delete('folder/id')
    expect(fetchMock.mock.calls[5]?.[0]).toBe('/api/v1/folders/folder%2Fid')
  })

  it('uses the Trash batch, Trash page, and empty Trash HTTP contracts', async () => {
    const fetchMock = vi.fn()
      .mockResolvedValueOnce({
        ok: true,

        /**
         * Return the JSON response payload configured for this adapter test.
         */
        json: async () => ({ trashed: 2 })
      })
      .mockResolvedValueOnce({
        ok: true,

        /**
         * Return the JSON response payload configured for this adapter test.
         */
        json: async () => [{ id: 'mail-id', seq: 4, subject: 'Old', from: [], to: [], received_at: 1, size: 3, attachment_count: 0 }]
      })
      .mockResolvedValueOnce({
        ok: true,

        /**
         * Return the JSON response payload configured for this adapter test.
         */
        json: async () => []
      })
      .mockResolvedValueOnce({
        ok: true,

        /**
         * Return the JSON response payload configured for this adapter test.
         */
        json: async () => ({ deleted: 5 })
      })
    vi.stubGlobal('fetch', fetchMock)
    const client = createHttpClient()
    expect(await client.messages.trash(['one', 'two'])).toBe(2)
    expect(fetchMock.mock.calls[0]?.[0]).toBe('/api/v1/messages/trash')
    expect(fetchMock.mock.calls[0]?.[1]).toMatchObject({ method: 'POST', body: JSON.stringify({ identifiers: ['one', 'two'] }) })
    expect(await client.trash.listMessages({ before: 9, limit: 4, query: 'subject == "x"' })).toMatchObject([{ id: 'mail-id', sequence: 4 }])
    expect(fetchMock.mock.calls[1]?.[0]).toBe('/api/v1/trash/messages?before=9&limit=4&q=subject+%3D%3D+%22x%22')
    await client.trash.listMessages()
    expect(fetchMock.mock.calls[2]?.[0]).toBe('/api/v1/trash/messages')
    expect(await client.trash.empty()).toBe(5)
    expect(fetchMock.mock.calls[3]?.[0]).toBe('/api/v1/trash')
    expect(fetchMock.mock.calls[3]?.[1]).toMatchObject({ method: 'DELETE' })
  })

  it('routes the message list folder selector to the manual folder endpoint', async () => {
    const fetchMock = vi.fn().mockResolvedValue({
      ok: true,

      /**
       * Return the JSON response payload configured for this adapter test.
       */
      json: async () => []
    })
    vi.stubGlobal('fetch', fetchMock)
    await createHttpClient().messages.list({ folderId: 'folder/id', before: 12, limit: 5, query: 'subject == "x"' })
    expect(fetchMock).toHaveBeenCalledWith(
      '/api/v1/folders/folder%2Fid/messages?before=12&limit=5&q=subject+%3D%3D+%22x%22',
      expect.objectContaining({ credentials: 'same-origin' }),
    )
  })

  it('keeps canonical global query listing unscoped so the server can include filed mail', async () => {
    const fetchMock = vi.fn().mockResolvedValue({
      ok: true,

      /**
       * Return the JSON response payload configured for this adapter test.
       */
      json: async () => []
    })
    vi.stubGlobal('fetch', fetchMock)
    await createHttpClient().messages.list({ query: 'true', limit: 8 })
    expect(fetchMock).toHaveBeenCalledWith('/api/v1/messages?limit=8&q=true', expect.objectContaining({ credentials: 'same-origin' }))
  })

  it('quotes literal searches without converting control characters into another DSL value', async () => {
    const fetchMock = vi.fn().mockResolvedValue({
      ok: true,

      /**
       * Return the JSON response payload configured for this adapter test.
       */
      json: async () => []
    })
    vi.stubGlobal('fetch', fetchMock)
    const query = '\" or true or \"' + String.fromCharCode(1) + '\\folder'
    await createHttpClient().messages.search(query)
    const parameters = new URL(fetchMock.mock.calls[0]![0], 'http://localhost').searchParams
    expect(parameters.get('q')).toBe('content contains "\\" or true or \\"' + String.fromCharCode(1) + '\\\\folder"')
  })

  it('requires complete valid inbox addresses and rejects malformed values locally', async () => {
    const fetchMock = vi.fn()
    vi.stubGlobal('fetch', fetchMock)
    const create = createHttpClient().inboxes.create
    await expect(create({ name: ' ', address: 'valid@example.test' })).rejects.toThrow('Invalid inbox name or address')
    for (const address of [
      'local', 'a@@example.test', 'a b@example.test', '"a"@example.test', 'a\\b@example.test',
      '.a@example.test', 'a..b@example.test', 'a.@example.test', 'a@-example.test',
      'a@example-.test', 'a@example..test', `${'a'.repeat(65)}@example.test`,
      `a@${'x'.repeat(64)}.test`, `${'a'.repeat(64)}@${'x'.repeat(186)}.test`,
      '\ta@example.test', 'a@example.test\r', 'a@exa\r\nmple.test', 'a@exämple.test',
    ]) {
      await expect(create({ name: 'Inbox', address })).rejects.toThrow('Invalid inbox name or address')
    }
    expect(fetchMock).not.toHaveBeenCalled()
  })

  it('closes the stream and removes all handlers on cleanup', async () => {
    const removed: string[] = []
    const handlers = new Map<string, EventListener>()
    const stream = {
      addEventListener: vi.fn((name: string, handler: EventListener) => handlers.set(name, handler)),
      removeEventListener: vi.fn((name: string) => removed.push(name)),
      close: vi.fn(),
      onerror: null as ((event: Event) => void) | null,
    }
    vi.stubGlobal('EventSource', vi.fn(function MockEventSource() { return stream }))
    const stop = await createHttpClient().events.subscribe(() => { })
    stop()
    expect(stream.close).toHaveBeenCalledOnce()
    expect(removed.sort()).toEqual(['message', 'ready', 'resync'])
    expect(stream.onerror).toBeNull()
  })

  it('maps well-formed events and requests resync for malformed message payloads', async () => {
    const handlers = new Map<string, EventListener>()
    const stream = {
      addEventListener: vi.fn((name: string, handler: EventListener) => handlers.set(name, handler)),
      removeEventListener: vi.fn(), close: vi.fn(), onerror: null as ((event: Event) => void) | null,
    }
    vi.stubGlobal('EventSource', vi.fn(function MockEventSource() { return stream }))
    const listener = vi.fn()
    await createHttpClient().events.subscribe(listener)

    /**
     * Deliver a synthetic event to the registered event-stream handler.
     */
    const dispatch = (type: string, data = '') => handlers.get(type)!(new MessageEvent(type, { data }))
    dispatch('ready')
    dispatch('message', JSON.stringify({ id: 'mail-id', seq: 19 }))
    dispatch('message', '{broken')
    dispatch('message', JSON.stringify({ id: 3, seq: '19' }))
    dispatch('message', JSON.stringify({ id: 'mail-id', seq: 1.5 }))
    dispatch('resync')
    expect(listener.mock.calls.map(([event]) => event)).toEqual([
      { type: 'ready' },
      { type: 'message', id: 'mail-id', sequence: 19 },
      { type: 'resync' }, { type: 'resync' }, { type: 'resync' }, { type: 'resync' },
    ])
  })

  it('sends administration requests to the versioned routes with Rust field names', async () => {
    const account = { identifier: 'user-id', name: 'Reader', email: 'reader@example.test', role: 'viewer', mail_access: 'scoped', personal_filter: null }
    const fetchMock = vi.fn()
      .mockResolvedValueOnce({
        ok: true,

        /**
         * Return the JSON response payload configured for this adapter test.
         */
        json: async () => account
      })
      .mockResolvedValueOnce({
        ok: true,

        /**
         * Return the JSON response payload configured for this adapter test.
         */
        json: async () => ({ ...account, mail_access: 'all' })
      })
      .mockResolvedValueOnce({
        ok: true,

        /**
         * Return the JSON response payload configured for this adapter test.
         */
        json: async () => []
      })
      .mockResolvedValueOnce({
        ok: false, status: 409,

        /**
         * Expose a response clone so the HTTP adapter can read the mocked error payload.
         */
        clone: () => ({
          /**
           * Return the JSON response payload configured for this adapter test.
           */
          json: async () => ({ error: 'scope still has child scopes' })
        }),
      })
    vi.stubGlobal('fetch', fetchMock)
    const client = createHttpClient()
    expect(await client.admin.users.create({ name: 'Reader', email: 'reader@example.test', password: 'reader password', role: 'viewer', mailAccess: 'scoped' }))
      .toEqual({ id: 'user-id', name: 'Reader', email: 'reader@example.test', role: 'viewer', mailAccess: 'scoped' })
    expect((await client.admin.users.update('user-id', { mailAccess: 'all' })).mailAccess).toBe('all')
    await client.admin.scopes.preview({ parentId: null, filter: 'subject contains "x"' })
    await expect(client.admin.scopes.delete('scope-id')).rejects.toMatchObject({ status: 409, detail: 'scope still has child scopes' })
    expect(fetchMock.mock.calls.map(([path, init]) => `${init.method} ${path}`)).toEqual([
      'POST /api/v1/users', 'PUT /api/v1/users/user-id', 'POST /api/v1/scopes/preview', 'DELETE /api/v1/scopes/scope-id',
    ])
    expect(JSON.parse(fetchMock.mock.calls[0]![1].body)).toEqual({
      name: 'Reader', email: 'reader@example.test', password: 'reader password', role: 'viewer', mail_access: 'scoped',
    })
    expect(JSON.parse(fetchMock.mock.calls[1]![1].body)).toEqual({ mail_access: 'all' })
    expect(JSON.parse(fetchMock.mock.calls[2]![1].body)).toEqual({ parent: null, filter: 'subject contains "x"', limit: 20 })
  })

  it('sends SMTP access administration to the versioned routes', async () => {
    const access = {
      identifier: 'access-id', name: 'Staging', username: 'staging',
      enabled: true, created_at: 10, last_used_at: null,
    }
    const issued = { access, password: 'shown-once', server: { listen_address: '127.0.0.1:1025', listener: null } }
    const fetchMock = vi.fn()
      .mockResolvedValueOnce({
        ok: true,

        /**
         * Return the JSON response payload configured for this adapter test.
         */
        json: async () => [access]
      })
      .mockResolvedValueOnce({
        ok: true,

        /**
         * Return the JSON response payload configured for this adapter test.
         */
        json: async () => issued
      })
      .mockResolvedValueOnce({
        ok: true,

        /**
         * Return the JSON response payload configured for this adapter test.
         */
        json: async () => issued
      })
      .mockResolvedValueOnce({
        ok: true,

        /**
         * Return the JSON response payload configured for this adapter test.
         */
        json: async () => ({ ...access, enabled: false })
      })
      .mockResolvedValueOnce({
        ok: true,

        /**
         * Return the JSON response payload configured for this adapter test.
         */
        json: async () => ({ ...access, requires_encryption: true, allowed_mechanisms: ['login'] })
      })
      .mockResolvedValueOnce({ ok: true, status: 204 })
      .mockResolvedValueOnce({
        ok: false, status: 403,

        /**
         * Expose a response clone so the HTTP adapter can read the mocked error payload.
         */
        clone: () => ({
          /**
           * Return the JSON response payload configured for this adapter test.
           */
          json: async () => ({ error: 'forbidden' })
        })
      })
    vi.stubGlobal('fetch', fetchMock)
    const client = createHttpClient()
    expect((await client.admin.smtp.list())[0]).toMatchObject({ id: 'access-id', createdAt: 10_000, lastUsedAt: null })
    expect((await client.admin.smtp.create({ name: 'Staging', username: 'staging' })).password).toBe('shown-once')
    expect((await client.admin.smtp.regeneratePassword('access-id')).password).toBe('shown-once')
    expect((await client.admin.smtp.setEnabled('access-id', false)).enabled).toBe(false)
    expect(await client.admin.smtp.updateRules('access-id', { requiresEncryption: true, allowedMechanisms: ['login'] }))
      .toMatchObject({ requiresEncryption: true, allowedMechanisms: ['login'] })
    await client.admin.smtp.delete('access-id')
    await expect(client.admin.smtp.server()).rejects.toMatchObject({ status: 403 })
    expect(fetchMock.mock.calls.map(([path, init]) => `${init?.method ?? 'GET'} ${path}`)).toEqual([
      'GET /api/v1/smtp/accesses', 'POST /api/v1/smtp/accesses', 'POST /api/v1/smtp/accesses/access-id/password',
      'PUT /api/v1/smtp/accesses/access-id', 'PUT /api/v1/smtp/accesses/access-id', 'DELETE /api/v1/smtp/accesses/access-id',
      'GET /api/v1/smtp/server',
    ])
    expect(JSON.parse(fetchMock.mock.calls[1]![1].body)).toEqual({ name: 'Staging', username: 'staging' })
    expect(JSON.parse(fetchMock.mock.calls[3]![1].body)).toEqual({ enabled: false })
    expect(JSON.parse(fetchMock.mock.calls[4]![1].body)).toEqual({ requires_encryption: true, allowed_mechanisms: ['login'] })
  })

  it.each([true, false])('sets primary Seen to %s through the shared API and returns changed count', async (seen) => {
    const fetchMock = vi.fn(async () => ({
      ok: true,

      /**
       * Return the JSON response payload configured for this adapter test.
       */
      json: async () => ({ changed: 2 })
    }));
    vi.stubGlobal('fetch', fetchMock);
    expect(await createHttpClient().messages.setSeen(['a', 'b'], seen)).toBe(2);
    expect(fetchMock).toHaveBeenCalledWith('/api/v1/messages/seen', expect.objectContaining({
      method: 'POST', body: JSON.stringify({ identifiers: ['a', 'b'], seen }),
    }));
  });

  it('sends IMAP account management to the user-level versioned routes', async () => {
    const folder = {
      identifier: 'folder-id', account_identifier: 'account-id', kind: 'private', definition_identifier: null, name: 'Signups', filter: '', include_in_inbox: true,
      created_at: 10, updated_at: 11,
    }
    const account = {
      identifier: 'account-id', owner_identifier: 'user-id', name: 'Thunderbird', username: 'tb', enabled: true, mode: 'mirror', mirror_views: true,
      created_at: 10, updated_at: 11, last_used_at: 12, linked_views: [], folders: [folder],
      mailboxes: [{ name: 'INBOX', source: 'inbox', view_identifier: null, folder_identifier: null }],
    }
    const server = { enabled: true, host: '127.0.0.1', port: 1143, security: 'start_tls_required', listen_address: '127.0.0.1:1143' }
    const issued = { account, password: 'shown-once', server }
    const fetchMock = vi.fn()
      .mockResolvedValueOnce({
        ok: true,

        /**
         * Return the JSON response payload configured for this adapter test.
         */
        json: async () => server
      })
      .mockResolvedValueOnce({
        ok: true,

        /**
         * Return the JSON response payload configured for this adapter test.
         */
        json: async () => [account]
      })
      .mockResolvedValueOnce({
        ok: true, status: 201,

        /**
         * Return the JSON response payload configured for this adapter test.
         */
        json: async () => issued
      })
      .mockResolvedValueOnce({
        ok: true,

        /**
         * Return the JSON response payload configured for this adapter test.
         */
        json: async () => account
      })
      .mockResolvedValueOnce({
        ok: true,

        /**
         * Return the JSON response payload configured for this adapter test.
         */
        json: async () => ({ ...account, enabled: false })
      })
      .mockResolvedValueOnce({
        ok: true,

        /**
         * Return the JSON response payload configured for this adapter test.
         */
        json: async () => issued
      })
      .mockResolvedValueOnce({
        ok: true,

        /**
         * Return the JSON response payload configured for this adapter test.
         */
        json: async () => ({ restored: 2 })
      })
      .mockResolvedValueOnce({
        ok: true, status: 201,

        /**
         * Return the JSON response payload configured for this adapter test.
         */
        json: async () => folder
      })
      .mockResolvedValueOnce({
        ok: true,

        /**
         * Return the JSON response payload configured for this adapter test.
         */
        json: async () => ({ ...folder, include_in_inbox: false })
      })
      .mockResolvedValueOnce({ ok: true, status: 204 })
      .mockResolvedValueOnce({ ok: true, status: 204 })
      .mockResolvedValueOnce({
        ok: false, status: 409,

        /**
         * Expose a response clone so the HTTP adapter can read the mocked error payload.
         */
        clone: () => ({
          /**
           * Return the JSON response payload configured for this adapter test.
           */
          json: async () => ({ error: 'conflict' })
        })
      })
    vi.stubGlobal('fetch', fetchMock)
    const client = createHttpClient()
    expect(await client.imap.server()).toEqual({
      enabled: true, host: '127.0.0.1', port: 1143, security: 'startTlsRequired', listenAddress: '127.0.0.1:1143',
    })
    expect((await client.imap.list())[0]).toMatchObject({ id: 'account-id', createdAt: 10_000, updatedAt: 11_000, lastUsedAt: 12_000 })
    expect((await client.imap.create({ name: 'Thunderbird', username: 'tb' })).password).toBe('shown-once')
    expect((await client.imap.get('account-id')).folders[0]).toMatchObject({ id: 'folder-id', accountId: 'account-id', createdAt: 10_000 })
    expect((await client.imap.update('account-id', { enabled: false, mode: 'independent' })).enabled).toBe(false)
    expect((await client.imap.regeneratePassword('account-id')).password).toBe('shown-once')
    expect(await client.imap.restoreHidden('account-id')).toBe(2)
    expect((await client.imap.folders.create('account-id', { name: 'Signups', filter: '' })).includeInInbox).toBe(true)
    expect((await client.imap.folders.update('account-id', 'folder-id', { name: 'Signups', filter: '', includeInInbox: false })).includeInInbox).toBe(false)
    await client.imap.folders.delete('account-id', 'folder-id')
    await client.imap.delete('account-id')
    await expect(client.imap.folders.create('account-id', { name: 'Signups', filter: '' })).rejects.toMatchObject({ status: 409, detail: 'conflict' })
    expect(fetchMock.mock.calls.map(([path, init]) => `${init?.method ?? 'GET'} ${path}`)).toEqual([
      'GET /api/v1/imap/server', 'GET /api/v1/imap/accounts', 'POST /api/v1/imap/accounts', 'GET /api/v1/imap/accounts/account-id',
      'PUT /api/v1/imap/accounts/account-id', 'POST /api/v1/imap/accounts/account-id/password', 'POST /api/v1/imap/accounts/account-id/restore',
      'POST /api/v1/imap/accounts/account-id/folders', 'PUT /api/v1/imap/accounts/account-id/folders/folder-id',
      'DELETE /api/v1/imap/accounts/account-id/folders/folder-id', 'DELETE /api/v1/imap/accounts/account-id',
      'POST /api/v1/imap/accounts/account-id/folders',
    ])
    expect(JSON.parse(fetchMock.mock.calls[2]![1].body)).toEqual({ name: 'Thunderbird', username: 'tb', mode: 'mirror' })
    expect(JSON.parse(fetchMock.mock.calls[4]![1].body)).toEqual({ enabled: false, mode: 'independent' })
    expect(fetchMock.mock.calls[5]![1].body).toBeUndefined()
    expect(JSON.parse(fetchMock.mock.calls[7]![1].body)).toEqual({ name: 'Signups', filter: '', include_in_inbox: true })
    expect(JSON.parse(fetchMock.mock.calls[8]![1].body)).toEqual({ name: 'Signups', filter: '', include_in_inbox: false })
  })

  it('copies primary mail and updates supported flags through their versioned endpoints', async () => {
    const fetchMock = vi.fn()
      .mockResolvedValueOnce({
        ok: true,

        /**
         * Return the JSON response payload configured for this adapter test.
         */
        json: async () => ({ copied: 2 })
      })
      .mockResolvedValueOnce({
        ok: true,

        /**
         * Return the JSON response payload configured for this adapter test.
         */
        json: async () => ({ changed: 1 })
      })
    vi.stubGlobal('fetch', fetchMock)
    const client = createHttpClient()
    expect(await client.messages.copy(['a', 'b'], 'folder-id')).toBe(2)
    expect(await client.messages.setFlags({
      identifiers: ['a'], operation: 'add',
      flags: { seen: false, answered: false, flagged: true, draft: false, deleted: false, keywords: ['important'] },
    })).toBe(1)
    expect(fetchMock.mock.calls.map(([path, init]) => [path, init?.method, JSON.parse(String(init?.body))])).toEqual([
      ['/api/v1/messages/copy', 'POST', { identifiers: ['a', 'b'], folderIdentifier: 'folder-id' }],
      ['/api/v1/messages/flags', 'POST', {
        identifiers: ['a'], operation: 'add', flags: { seen: false, answered: false, flagged: true, draft: false, deleted: false, keywords: ['important'] },
      }],
    ])
  })

  it('maps Persona operations to the versioned Persona routes', async () => {
    const persona = {
      id: 'persona/1', owner_id: 'user', name: 'Alice', address: 'alice@example.test', icon: 'bot', script: 'open;', enabled: true,
      revision: 1, created_at: 1, updated_at: 1, address_access: 'confirmed', last_execution: null,
    }
    const responses: unknown[] = [
      persona, { ...persona, icon: null }, { ...persona, enabled: false },
      { valid: true, error: null, statements: 1, longest_wait_milliseconds: 0 },
      [], [{ id: 'run', seq: 4, persona_id: 'persona/1', message_id: 'm', status: 'running', scheduled_at: 1, started_at: 2, completed_at: null, persona_revision: 1, error_message: null, subject: 'S' }],
    ]
    const fetchMock = vi.fn(async () => ({ ok: true, status: 200, json: async () => responses.shift() }))
    vi.stubGlobal('fetch', fetchMock)
    const client = createHttpClient()
    const definition = { name: 'Alice', address: 'alice@example.test', script: 'open;', enabled: true }
    const created = await client.personas!.create({ ...definition, icon: 'bot' })
    expect(created).toMatchObject({ ownerId: 'user', addressAccess: 'confirmed', icon: 'bot' })
    expect((await client.personas!.update('persona/1', { ...definition, icon: null })).icon).toBeNull()
    expect((await client.personas!.setEnabled('persona/1', false)).enabled).toBe(false)
    expect((await client.personas!.validate('open;')).statements).toBe(1)
    await client.personas!.listMessages('persona/1', { before: 9, limit: 20, query: 'subject contains "x"' })
    const runs = await client.personas!.executions('persona/1', { limit: 5 })
    expect(runs[0]).toMatchObject({ id: 'run', sequence: 4, status: 'running', completedAt: null })
    const calls = fetchMock.mock.calls as unknown as [string, RequestInit][]
    expect(calls.map(([path, init]) => [init?.method ?? 'GET', path])).toEqual([
      ['POST', '/api/v1/personas'],
      ['PUT', '/api/v1/personas/persona%2F1'],
      ['PUT', '/api/v1/personas/persona%2F1/enabled'],
      ['POST', '/api/v1/personas/validate'],
      ['GET', '/api/v1/personas/persona%2F1/messages?before=9&limit=20&q=subject+contains+%22x%22'],
      ['GET', '/api/v1/personas/persona%2F1/executions?limit=5'],
    ])
    expect(JSON.parse(String(calls[0]![1].body))).toEqual({ ...definition, icon: 'bot' })
    expect(JSON.parse(String(calls[1]![1].body))).toEqual({ ...definition, icon: null })
    expect(JSON.parse(String(calls[2]![1].body))).toEqual({ enabled: false })
  })
})
