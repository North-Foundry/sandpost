import type {
  InstanceStatus,
  LoginInput,
  HtmlCompatibilityReport,
  MessageDetail,
  MessageSummary,
  SandpostClient,
  SetupInput,
} from '@/api/contracts'
import type {
  WireAccount,
  WireImapAccount,
  WireImapLocalFolder,
  WireImapServer,
  WireInbox,
  WireIssuedImapCredentials,
  WireIssuedSmtpCredentials,
  WireBulkLinkCheckResult,
  WireCompatibilityReport,
  WireLinkCheckReport,
  WireMessageDetail,
  WireMessageSummary,
  WireQueryValidation,
  WireScope,
  WireSessionUser,
  WireSmtpAccess,
  WireSmtpServer,
  WireView,
  WireWebFolder,
} from '@/api/wire'
import {
  normalizeBulkLinkCheck,
  normalizeCompatibilityReport,
  normalizeDetail,
  normalizeLinkCheckReport,
  normalizeSummary,
  normalizeView,
  normalizeWebFolder,
  viewWire,
} from '@/api/normalize'
import {
  accountUpdateWire,
  newAccountWire,
  normalizeAccount,
  normalizeIssuedCredentials,
  normalizeScope,
  normalizeSmtpAccess,
  normalizeSmtpServer,
  normalizeValidation,
  scopePreviewWire,
  scopeWire,
  smtpAccessRulesWire,
  smtpAccessWire,
} from '@/api/admin'
import {
  imapAccountUpdateWire,
  imapAccountWire,
  imapFolderWire,
  normalizeImapAccount,
  normalizeImapFolder,
  normalizeImapServer,
  normalizeIssuedImapCredentials,
} from '@/api/imap'
import { normalizeInboxAddress } from '@/api/inboxAddress'
import {
  normalizePersona,
  normalizePersonaExecution,
  normalizePersonaExecutionDetails,
  normalizePersonaScriptValidation,
  personaWire,
} from '@/api/personas'
import type {
  WirePersona,
  WirePersonaExecution,
  WirePersonaScriptValidation,
} from '@/api/wire'

/**
 * Adapt the local versioned HTTP API and its event stream.
 */
export function createHttpClient(): SandpostClient {
  let token: string | null = null

  /**
   * Send an authenticated same-origin HTTP request and attach status details to failed responses.
   */
  const request = async (
    path: string,
    init?: RequestInit
  ): Promise<Response> => {
    const headers = new Headers(init?.headers)
    if (token) headers.set('Authorization', `Bearer ${token}`)
    const response = await fetch(path, {
      ...init,
      headers,
      credentials: 'same-origin',
    })
    if (!response.ok) {
      const body = await response
        .clone()
        .json()
        .catch(() => null) as { error?: unknown; code?: unknown } | null
      const detail = typeof body?.error === 'string' ? `: ${body.error}` : ''
      const code = typeof body?.code === 'string' ? ` [${body.code}]` : ''
      throw Object.assign(
        new Error(`Sandpost API ${response.status}${code}${detail}: ${path}`),
        {
          status: response.status,
          code: typeof body?.code === 'string' ? body.code : body?.error,
          detail: typeof body?.error === 'string' ? body.error : undefined,
        }
      )
    }
    return response
  }

  /**
   * Send a JSON body and decode a JSON response.
   */
  const sendJson = async <T>(
    method: string,
    path: string,
    body?: unknown
  ): Promise<T> => {
    const response = await request(path, {
      method,
      ...(body === undefined
        ? {}
        : {
          headers: { 'Content-Type': 'application/json' },
          body: JSON.stringify(body),
        }),
    })
    return response.json() as Promise<T>
  }

  /**
   * Send a request whose successful response has no body.
   */
  const sendEmpty = async (
    method: string,
    path: string,
    body?: unknown
  ): Promise<void> => {
    await request(path, {
      method,
      ...(body === undefined
        ? {}
        : {
          headers: { 'Content-Type': 'application/json' },
          body: JSON.stringify(body),
        }),
    })
  }
  const segment = encodeURIComponent

  /**
   * Fetch message summaries and convert each wire row to the frontend contract.
   */
  const listMessages = async (path: string): Promise<MessageSummary[]> => {
    const rows = await (await request(path)).json() as WireMessageSummary[]
    return rows.map(normalizeSummary)
  }

  /**
   * Encode a keyset page and optional DSL narrowing as a query string, empty when nothing is set.
   */
  const pageSuffix = (options: {
    before?: number
    limit?: number
    query?: string
  }): string => {
    const query = new URLSearchParams()
    if (options.before !== undefined)
      query.set('before', String(options.before))
    if (options.limit !== undefined) query.set('limit', String(options.limit))
    if (options.query !== undefined) query.set('q', options.query)
    return query.size ? `?${query}` : ''
  }

  /**
   * Fetch a page of messages from a Web folder.
   */
  const listFolderMessages = async (
    id: string,
    options: { before?: number; limit?: number; query?: string } = {}
  ): Promise<MessageSummary[]> => {
    return listMessages(
      `/api/v1/folders/${segment(id)}/messages${pageSuffix(options)}`
    )
  }

  return {
    /**
     * Read whether initial setup is complete and whether a setup key is required.
     */
    async instanceStatus() {
      return (
        await request('/api/v1/instance')
      ).json() as Promise<InstanceStatus>
    },

    /**
     * Exchange the setup key for a temporary setup authorization.
     */
    async authorizeSetup(input) {
      return (
        await request('/api/v1/setup/authorize', {
          method: 'POST',
          headers: { 'Content-Type': 'application/json' },
          body: JSON.stringify(input),
        })
      ).json() as Promise<{ authorization: string }>
    },

    /**
     * Create the initial owner using the temporary bootstrap authorization.
     */
    async setup(input: SetupInput) {
      await request('/api/v1/setup', {
        method: 'POST',
        headers: { 'Content-Type': 'application/json' },
        body: JSON.stringify(input),
      })
    },

    /**
     * Authenticate the supplied credentials and return the signed-in user.
     */
    async login(input: LoginInput) {
      const result = await (
        await request('/api/v1/login', {
          method: 'POST',
          headers: { 'Content-Type': 'application/json' },
          body: JSON.stringify(input),
        })
      ).json() as { token: string; user: WireSessionUser }
      token = result.token
      return result.user
    },

    /**
     * Read the current session, returning null when it is no longer authenticated.
     */
    async session() {
      try {
        return await (await request('/api/v1/session')).json() as WireSessionUser
      } catch (error) {
        if (
          error &&
          typeof error === 'object' &&
          'status' in error &&
          error.status === 401
        ) {
          token = null
          return null
        }
        throw error
      }
    },

    /**
     * End the current authenticated session.
     */
    async logout() {
      try {
        await request('/api/v1/logout', { method: 'POST' })
      } finally {
        token = null
      }
    },
    inboxes: {
      /**
       * List the records available through this client method.
       */
      async list() {
        return (await request('/api/v1/inboxes')).json() as Promise<WireInbox[]>
      },

      /**
       * Create a record from the supplied input.
       */
      async create({ name, address }) {
        const normalizedName = name.trim()
        if (!normalizedName || normalizedName.length > 100) {
          throw new TypeError('Invalid inbox name or address')
        }
        const normalizedAddress = normalizeInboxAddress(address)
        const response = await request('/api/v1/inboxes', {
          method: 'POST',
          headers: { 'Content-Type': 'application/json' },
          body: JSON.stringify({
            name: normalizedName,
            address: normalizedAddress,
          }),
        })
        return response.json() as Promise<WireInbox>
      },
    },
    views: {
      /**
       * List the records available through this client method.
       */
      async list() {
        return (await sendJson<WireView[]>('GET', '/api/v1/views')).map(
          normalizeView
        )
      },

      /**
       * Save the supplied  definition.
       */
      async save(view) {
        return normalizeView(
          await sendJson<WireView>(
            'PUT',
            `/api/v1/views/${segment(view.id)}`,
            viewWire(view)
          )
        )
      },

      /**
       * Delete the record identified by the supplied ID.
       */
      delete: (id) => sendEmpty('DELETE', `/api/v1/views/${segment(id)}`),
    },
    folders: {
      /**
       * List the records available through this client method.
       */
      async list() {
        return (await sendJson<WireWebFolder[]>('GET', '/api/v1/folders')).map(
          normalizeWebFolder
        )
      },

      /**
       * Create a record from the supplied input.
       */
      async create(input) {
        return normalizeWebFolder(
          await sendJson<WireWebFolder>('POST', '/api/v1/folders', input)
        )
      },

      /**
       * Apply the supplied changes to the selected record.
       */
      async update(id, input) {
        return normalizeWebFolder(
          await sendJson<WireWebFolder>(
            'PUT',
            `/api/v1/folders/${segment(id)}`,
            input
          )
        )
      },

      /**
       * Delete the record identified by the supplied ID.
       */
      delete: (id) => sendEmpty('DELETE', `/api/v1/folders/${segment(id)}`),

      /**
       * Fetch message summaries and convert each wire row to the frontend contract.
       */
      async listMessages(id, options = {}) {
        return listFolderMessages(id, options)
      },
    },
    messages: {
      /**
       * List the records available through this client method.
       */
      async list(options = {}): Promise<MessageSummary[]> {
        if (options.folderId !== undefined)
          return listFolderMessages(options.folderId, options)
        const query = new URLSearchParams()
        if (options.before !== undefined)
          query.set('before', String(options.before))
        if (options.limit !== undefined)
          query.set('limit', String(options.limit))
        if (options.inboxId !== undefined)
          query.set('inbox_id', options.inboxId)
        if (options.viewId !== undefined) query.set('view_id', options.viewId)
        if (options.query !== undefined) query.set('q', options.query)
        const suffix = query.size ? `?${query}` : ''
        return listMessages(`/api/v1/messages${suffix}`)
      },

      /**
       * Search messages and return matching summaries.
       */
      async search(query, options = {}) {
        const parameters = new URLSearchParams({
          q: `content contains "${query.replace(/[\\"]/g, '\\$&')}"`,
        })
        if (options.before !== undefined)
          parameters.set('before', String(options.before))
        if (options.limit !== undefined)
          parameters.set('limit', String(options.limit))
        return listMessages(`/api/v1/search?${parameters}`)
      },

      /**
       * Set the Seen flag for the selected messages and return the number changed.
       */
      async setSeen(identifiers, seen) {
        return Number(
          (
            await sendJson<{ changed: number }>(
              'POST',
              '/api/v1/messages/seen',
              { identifiers, seen }
            )
          ).changed
        )
      },

      /**
       * Copy the selected messages to the requested folder.
       */
      async copy(identifiers, folderIdentifier) {
        return Number(
          (
            await sendJson<{ copied: number }>(
              'POST',
              '/api/v1/messages/copy',
              { identifiers, folderIdentifier }
            )
          ).copied
        )
      },

      /**
       * Apply the requested flag operation to the selected messages.
       */
      async setFlags(input) {
        return Number(
          (
            await sendJson<{ changed: number }>(
              'POST',
              '/api/v1/messages/flags',
              input
            )
          ).changed
        )
      },

      /**
       * Retrieve the record identified by the supplied ID.
       */
      async get(id): Promise<MessageDetail> {
        const detail = await (
          await request(`/api/v1/messages/${encodeURIComponent(id)}`)
        ).json() as WireMessageDetail
        return normalizeDetail(detail)
      },

      /**
       * Perform the get raw operation.
       */
      async getRaw(id): Promise<Uint8Array> {
        return new Uint8Array(
          await (
            await request(`/api/v1/messages/${encodeURIComponent(id)}/raw`)
          ).arrayBuffer()
        )
      },

      /**
       * Request the compatibility report of the message's original HTML.
       */
      async htmlCheck(id): Promise<HtmlCompatibilityReport> {
        const report = await (
          await request(`/api/v1/messages/${encodeURIComponent(id)}/html-check`)
        ).json() as WireCompatibilityReport
        return normalizeCompatibilityReport(report)
      },

      /**
       * Read the message's stored link check without starting one.
       */
      async linkCheck(id) {
        return normalizeLinkCheckReport(
          await sendJson<WireLinkCheckReport>(
            'GET',
            `/api/v1/messages/${encodeURIComponent(id)}/link-check`
          )
        )
      },

      /**
       * Start, or join, a background check of the message's links.
       */
      async startLinkCheck(id) {
        return normalizeLinkCheckReport(
          await sendJson<WireLinkCheckReport>(
            'POST',
            `/api/v1/messages/${encodeURIComponent(id)}/link-check`
          )
        )
      },

      /**
       * Start background link checks of the selected messages.
       */
      async checkLinks(identifiers) {
        return normalizeBulkLinkCheck(
          await sendJson<WireBulkLinkCheckResult>(
            'POST',
            '/api/v1/messages/link-check',
            { identifiers }
          )
        )
      },

      /**
       * Delete the record identified by the supplied ID.
       */
      async delete(id) {
        await request(`/api/v1/messages/${encodeURIComponent(id)}`, {
          method: 'DELETE',
        })
      },

      /**
       * Move the selected messages to the requested folder.
       */
      async move(identifiers, folderIdentifier) {
        const result = await sendJson<{ moved: number }>(
          'POST',
          '/api/v1/messages/placement',
          {
            identifiers,
            folderIdentifier,
          }
        )
        return Number(result.moved)
      },

      /**
       * Move the selected messages to Trash.
       */
      async trash(identifiers) {
        const result = await sendJson<{ trashed: number }>(
          'POST',
          '/api/v1/messages/trash',
          { identifiers }
        )
        return Number(result.trashed)
      },

      /**
       * Perform the get attachment operation.
       */
      async getAttachment(id, index) {
        return new Uint8Array(
          await (
            await request(
              `/api/v1/messages/${encodeURIComponent(id)}/attachments/${index}`
            )
          ).arrayBuffer()
        )
      },
    },
    trash: {
      /**
       * Fetch message summaries and convert each wire row to the frontend contract.
       */
      async listMessages(options = {}) {
        return listMessages(`/api/v1/trash/messages${pageSuffix(options)}`)
      },

      /**
       * Permanently remove messages currently held in Trash and return the removed count.
       */
      async empty() {
        return Number(
          (await sendJson<{ deleted: number }>('DELETE', '/api/v1/trash'))
            .deleted
        )
      },
    },
    personas: {
      /**
       * List the caller's own Personas.
       */
      async list() {
        return (await sendJson<WirePersona[]>('GET', '/api/v1/personas')).map(
          normalizePersona
        )
      },

      /**
       * Read one Persona.
       */
      async get(id) {
        return normalizePersona(
          await sendJson<WirePersona>('GET', `/api/v1/personas/${segment(id)}`)
        )
      },

      /**
       * Create a Persona owned by the caller.
       */
      async create(input) {
        return normalizePersona(
          await sendJson<WirePersona>(
            'POST',
            '/api/v1/personas',
            personaWire(input)
          )
        )
      },

      /**
       * Replace the caller's own Persona definition.
       */
      async update(id, input) {
        return normalizePersona(
          await sendJson<WirePersona>(
            'PUT',
            `/api/v1/personas/${segment(id)}`,
            personaWire(input)
          )
        )
      },

      /**
       * Enable or disable a Persona.
       */
      async setEnabled(id, enabled) {
        return normalizePersona(
          await sendJson<WirePersona>(
            'PUT',
            `/api/v1/personas/${segment(id)}/enabled`,
            { enabled }
          )
        )
      },

      /**
       * Delete a Persona and its history.
       */
      delete: (id) => sendEmpty('DELETE', `/api/v1/personas/${segment(id)}`),

      /**
       * Validate PersonaScript without saving it.
       */
      async validate(script) {
        return normalizePersonaScriptValidation(
          await sendJson<WirePersonaScriptValidation>(
            'POST',
            '/api/v1/personas/validate',
            { script }
          )
        )
      },

      /**
       * List readable mail addressed to a Persona.
       */
      async listMessages(id, options = {}) {
        return listMessages(
          `/api/v1/personas/${segment(id)}/messages${pageSuffix(options)}`
        )
      },

      /**
       * List a Persona's visible executions, newest first.
       */
      async executions(id, options = {}) {
        return (
          await sendJson<WirePersonaExecution[]>(
            'GET',
            `/api/v1/personas/${segment(id)}/executions${pageSuffix(options)}`
          )
        ).map(normalizePersonaExecution)
      },

      /**
       * Read one execution with its recorded steps.
       */
      async execution(id, executionId) {
        return normalizePersonaExecutionDetails(
          await sendJson<WirePersonaExecution>(
            'GET',
            `/api/v1/personas/${segment(id)}/executions/${segment(executionId)}`
          )
        )
      },
    },
    imap: {
      /**
       * Read the IMAP server connection details.
       */
      async server() {
        return normalizeImapServer(
          await sendJson<WireImapServer>('GET', '/api/v1/imap/server')
        )
      },

      /**
       * List the records available through this client method.
       */
      async list() {
        return (
          await sendJson<WireImapAccount[]>('GET', '/api/v1/imap/accounts')
        ).map(normalizeImapAccount)
      },

      /**
       * Retrieve the record identified by the supplied ID.
       */
      async get(id) {
        return normalizeImapAccount(
          await sendJson<WireImapAccount>(
            'GET',
            `/api/v1/imap/accounts/${segment(id)}`
          )
        )
      },

      /**
       * Create a record from the supplied input.
       */
      async create(input) {
        return normalizeIssuedImapCredentials(
          await sendJson<WireIssuedImapCredentials>(
            'POST',
            '/api/v1/imap/accounts',
            imapAccountWire(input)
          )
        )
      },

      /**
       * Apply the supplied changes to the selected record.
       */
      async update(id, changes) {
        return normalizeImapAccount(
          await sendJson<WireImapAccount>(
            'PUT',
            `/api/v1/imap/accounts/${segment(id)}`,
            imapAccountUpdateWire(changes)
          )
        )
      },

      /**
       * Delete the record identified by the supplied ID.
       */
      delete: (id) =>
        sendEmpty('DELETE', `/api/v1/imap/accounts/${segment(id)}`),

      /**
       * Issue a replacement credential and invalidate the previous password and sessions.
       */
      async regeneratePassword(id) {
        return normalizeIssuedImapCredentials(
          await sendJson<WireIssuedImapCredentials>(
            'POST',
            `/api/v1/imap/accounts/${segment(id)}/password`
          )
        )
      },

      /**
       * Restore messages hidden by mail-client expunges and return the restored count.
       */
      async restoreHidden(id) {
        return Number(
          (
            await sendJson<{ restored: number }>(
              'POST',
              `/api/v1/imap/accounts/${segment(id)}/restore`
            )
          ).restored
        )
      },
      folders: {
        /**
         * Create a record from the supplied input.
         */
        async create(accountId, input) {
          return normalizeImapFolder(
            await sendJson<WireImapLocalFolder>(
              'POST',
              `/api/v1/imap/accounts/${segment(accountId)}/folders`,
              imapFolderWire(input)
            )
          )
        },

        /**
         * Apply the supplied changes to the selected record.
         */
        async update(accountId, folderId, input) {
          return normalizeImapFolder(
            await sendJson<WireImapLocalFolder>(
              'PUT',
              `/api/v1/imap/accounts/${segment(accountId)}/folders/${segment(folderId)}`,
              imapFolderWire(input)
            )
          )
        },

        /**
         * Delete the record identified by the supplied ID.
         */
        delete: (accountId, folderId) =>
          sendEmpty(
            'DELETE',
            `/api/v1/imap/accounts/${segment(accountId)}/folders/${segment(folderId)}`
          ),
      },
    },
    admin: {
      smtp: {
        /**
         * Read the IMAP server connection details.
         */
        async server() {
          return normalizeSmtpServer(
            await sendJson<WireSmtpServer>('GET', '/api/v1/smtp/server')
          )
        },

        /**
         * List the records available through this client method.
         */
        async list() {
          return (
            await sendJson<WireSmtpAccess[]>('GET', '/api/v1/smtp/accesses')
          ).map(normalizeSmtpAccess)
        },

        /**
         * Create a record from the supplied input.
         */
        async create(input) {
          return normalizeIssuedCredentials(
            await sendJson<WireIssuedSmtpCredentials>(
              'POST',
              '/api/v1/smtp/accesses',
              smtpAccessWire(input)
            )
          )
        },

        /**
         * Issue a replacement credential and invalidate the previous password and sessions.
         */
        async regeneratePassword(id) {
          return normalizeIssuedCredentials(
            await sendJson<WireIssuedSmtpCredentials>(
              'POST',
              `/api/v1/smtp/accesses/${segment(id)}/password`
            )
          )
        },

        /**
         * Replace the access authentication rules.
         */
        async updateRules(id, rules) {
          return normalizeSmtpAccess(
            await sendJson<WireSmtpAccess>(
              'PUT',
              `/api/v1/smtp/accesses/${segment(id)}`,
              smtpAccessRulesWire(rules)
            )
          )
        },

        /**
         * Enable or disable the SMTP access.
         */
        async setEnabled(id, enabled) {
          return normalizeSmtpAccess(
            await sendJson<WireSmtpAccess>(
              'PUT',
              `/api/v1/smtp/accesses/${segment(id)}`,
              { enabled }
            )
          )
        },

        /**
         * Delete the record identified by the supplied ID.
         */
        delete: (id) =>
          sendEmpty('DELETE', `/api/v1/smtp/accesses/${segment(id)}`),
      },
      scopes: {
        /**
         * List the records available through this client method.
         */
        async list() {
          return (await sendJson<WireScope[]>('GET', '/api/v1/scopes')).map(
            normalizeScope
          )
        },

        /**
         * Save the supplied  definition.
         */
        async save(input) {
          return normalizeScope(
            await sendJson<WireScope>(
              'PUT',
              `/api/v1/scopes/${segment(input.id)}`,
              scopeWire(input)
            )
          )
        },

        /**
         * Delete the record identified by the supplied ID.
         */
        delete: (id) => sendEmpty('DELETE', `/api/v1/scopes/${segment(id)}`),

        /**
         * Preview messages matching the supplied scope definition.
         */
        async preview(input) {
          return (
            await sendJson<WireMessageSummary[]>(
              'POST',
              '/api/v1/scopes/preview',
              scopePreviewWire(input)
            )
          ).map(normalizeSummary)
        },

        /**
         * List user identifiers assigned to the selected scope.
         */
        assignments: (scopeId) =>
          sendJson<string[]>(
            'GET',
            `/api/v1/scopes/${segment(scopeId)}/assignments`
          ),

        /**
         * Assign the user to the selected scope.
         */
        assign: (scopeId, userId) =>
          sendEmpty(
            'PUT',
            `/api/v1/scopes/${segment(scopeId)}/assignments/${segment(userId)}`
          ),

        /**
         * Remove the user from the selected scope.
         */
        unassign: (scopeId, userId) =>
          sendEmpty(
            'DELETE',
            `/api/v1/scopes/${segment(scopeId)}/assignments/${segment(userId)}`
          ),
      },
      users: {
        /**
         * List the records available through this client method.
         */
        async list() {
          return (await sendJson<WireAccount[]>('GET', '/api/v1/users')).map(
            normalizeAccount
          )
        },

        /**
         * Create a record from the supplied input.
         */
        async create(input) {
          return normalizeAccount(
            await sendJson<WireAccount>(
              'POST',
              '/api/v1/users',
              newAccountWire(input)
            )
          )
        },

        /**
         * Apply the supplied changes to the selected record.
         */
        async update(id, changes) {
          return normalizeAccount(
            await sendJson<WireAccount>(
              'PUT',
              `/api/v1/users/${segment(id)}`,
              accountUpdateWire(changes)
            )
          )
        },

        /**
         * Delete the record identified by the supplied ID.
         */
        delete: (id) => sendEmpty('DELETE', `/api/v1/users/${segment(id)}`),
      },

      /**
       * Validate a query and return any compiler diagnostic.
       */
      async validateQuery(query) {
        return normalizeValidation(
          await sendJson<WireQueryValidation>(
            'POST',
            '/api/v1/query/validate',
            { query }
          )
        )
      },
    },
    events: {
      /**
       * Subscribe to runtime events and return a function that removes the subscription.
       */
      async subscribe(listener): Promise<() => void> {
        const source = new EventSource('/api/v1/events', {
          withCredentials: true,
        })

        /**
         * Create an event handler that validates message payloads and forwards stream events.
         */
        const handle =
          (type: 'ready' | 'resync' | 'message') =>
            (event: MessageEvent<string>) => {
              if (type !== 'message') return listener({ type })
              try {
                const payload: unknown = JSON.parse(event.data)
                if (!payload || typeof payload !== 'object')
                  throw new TypeError('Invalid message event')
                const { id, seq } = payload as { id?: unknown; seq?: unknown }
                if (
                  typeof id !== 'string' ||
                  typeof seq !== 'number' ||
                  !Number.isSafeInteger(seq)
                ) {
                  throw new TypeError('Invalid message event')
                }
                listener({ type, id, sequence: seq })
              } catch {
                listener({ type: 'resync' })
              }
            }
        const ready = handle('ready')
        const message = handle('message')
        const resync = handle('resync')

        /**
         * Notify subscribers when the event stream disconnects.
         */
        const disconnected = () => listener({ type: 'disconnected' })
        source.addEventListener('ready', ready as EventListener)
        source.addEventListener('message', message as EventListener)
        source.addEventListener('resync', resync as EventListener)
        source.onerror = disconnected
        return () => {
          source.removeEventListener('ready', ready as EventListener)
          source.removeEventListener('message', message as EventListener)
          source.removeEventListener('resync', resync as EventListener)
          source.onerror = null
          source.close()
        }
      },
    },
  }
}
