import type { SandpostDesktopBridge, SandpostDesktopPersonaBridge, WireDesktopEvent } from '@/api/desktop'
import type { SandpostClient, SandpostEvent, SandpostPersonaClient } from '@/api/contracts'
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
import {
  normalizePersona,
  normalizePersonaExecution,
  normalizePersonaExecutionDetails,
  normalizePersonaScriptValidation,
  personaWire,
} from '@/api/personas'

/**
 * Adapt the native Persona bridge to the shared client contract.
 */
function personaClient(bridge: SandpostDesktopPersonaBridge): SandpostPersonaClient {
  return {
    /**
     * List the caller's own Personas.
     */
    list: async () => (await nativeCall(() => bridge.list())).map(normalizePersona),

    /**
     * Read one Persona.
     */
    get: async (id) => normalizePersona(await nativeCall(() => bridge.get(id))),

    /**
     * Create a Persona owned by the caller.
     */
    create: async (input) => normalizePersona(await nativeCall(() => bridge.create(personaWire(input)))),

    /**
     * Replace the caller's own Persona definition.
     */
    update: async (id, input) => normalizePersona(await nativeCall(() => bridge.update(id, personaWire(input)))),

    /**
     * Enable or disable a Persona.
     */
    setEnabled: async (id, enabled) => normalizePersona(await nativeCall(() => bridge.setEnabled(id, enabled))),

    /**
     * Delete a Persona and its history.
     */
    delete: (id) => nativeCall(() => bridge.delete(id)),

    /**
     * Validate PersonaScript without saving it.
     */
    validate: async (script) => normalizePersonaScriptValidation(await nativeCall(() => bridge.validate(script))),

    /**
     * List readable mail addressed to a Persona.
     */
    listMessages: async (id, options = {}) => (await nativeCall(() => bridge.messages(id, options))).map(normalizeSummary),

    /**
     * List a Persona's visible executions.
     */
    executions: async (id, options = {}) => (await nativeCall(() => bridge.executions(id, options))).map(normalizePersonaExecution),

    /**
     * Read one execution with its recorded steps.
     */
    execution: async (id, executionId) => normalizePersonaExecutionDetails(await nativeCall(() => bridge.execution(id, executionId))),
  }
}

/**
 * Adapt the typed Electron bridge while preserving the shared Rust wire DTOs.
 */
export function createElectronClient(
  bridge: SandpostDesktopBridge
): SandpostClient {
  return {
    /**
     * Read whether initial setup is complete and whether a setup key is required.
     */
    instanceStatus: () => nativeCall(() => bridge.instanceStatus()),

    /**
     * Create the initial owner using the temporary bootstrap authorization.
     */
    setup: (input) => nativeCall(() => bridge.setup(input)),

    /**
     * Exchange the setup key for a temporary setup authorization.
     */
    authorizeSetup: async () => {
      throw new Error(
        'Setup key authorization is not required in the desktop app.'
      )
    },

    /**
     * Authenticate the supplied credentials and return the signed-in user.
     */
    login: async (input) => nativeCall(() => bridge.login(input)),

    /**
     * Read the current session, returning null when it is no longer authenticated.
     */
    session: async () => {
      const session = await nativeCall(() => bridge.session())
      return session
    },

    /**
     * End the current authenticated session.
     */
    logout: () => nativeCall(() => bridge.logout()),
    inboxes: {
      /**
       * List the records available through this client method.
       */
      list: () => bridge.inboxes.list(),

      /**
       * Create a record from the supplied input.
       */
      create: ({ name, address }) => bridge.inboxes.create(name, address),
    },
    views: {
      /**
       * List the records available through this client method.
       */
      list: async () =>
        (await nativeCall(() => bridge.views.list())).map(normalizeView),

      /**
       * Save the supplied  definition.
       */
      save: async (view) =>
        normalizeView(
          await nativeCall(() => bridge.views.save(viewWire(view)))
        ),

      /**
       * Delete the record identified by the supplied ID.
       */
      delete: (id) => nativeCall(() => bridge.views.delete(id)),
    },
    folders: {
      /**
       * List the records available through this client method.
       */
      async list() {
        return (await nativeCall(() => bridge.folders.list())).map(
          normalizeWebFolder
        )
      },

      /**
       * Create a record from the supplied input.
       */
      async create(input) {
        return normalizeWebFolder(
          await nativeCall(() => bridge.folders.create(input))
        )
      },

      /**
       * Apply the supplied changes to the selected record.
       */
      async update(id, input) {
        return normalizeWebFolder(
          await nativeCall(() => bridge.folders.update(id, input))
        )
      },

      /**
       * Delete the record identified by the supplied ID.
       */
      delete: (id) => nativeCall(() => bridge.folders.delete(id)),

      /**
       * Fetch message summaries and convert each wire row to the frontend contract.
       */
      async listMessages(id, options = {}) {
        return (
          await nativeCall(() => bridge.folders.listMessages(id, options))
        ).map(normalizeSummary)
      },
    },
    messages: {
      /**
       * List the records available through this client method.
       */
      async list(options) {
        if (options?.folderId) {
          return (
            await nativeCall(() =>
              bridge.folders.listMessages(options.folderId!, {
                before: options.before,
                limit: options.limit,
                query: options.query,
              })
            )
          ).map(normalizeSummary)
        }
        return (await bridge.messages.list(options)).map(normalizeSummary)
      },

      /**
       * Search messages and return matching summaries.
       */
      async search(query, options) {
        return (
          await bridge.messages.search(query, options?.before, options?.limit)
        ).map(normalizeSummary)
      },

      /**
       * Set the Seen flag for the selected messages and return the number changed.
       */
      setSeen: async (identifiers, seen) =>
        Number(
          (
            await nativeCall(() =>
              bridge.messages.setSeen({ identifiers, seen })
            )
          ).changed
        ),

      /**
       * Copy the selected messages to the requested folder.
       */
      copy: async (identifiers, folderIdentifier) =>
        Number(
          (
            await nativeCall(() =>
              bridge.messages.copy({ identifiers, folderIdentifier })
            )
          ).copied
        ),

      /**
       * Apply the requested flag operation to the selected messages.
       */
      setFlags: async (input) =>
        Number(
          (await nativeCall(() => bridge.messages.setFlags(input))).changed
        ),

      /**
       * Retrieve the record identified by the supplied ID.
       */
      async get(id) {
        return normalizeDetail(await bridge.messages.get(id))
      },

      /**
       * Perform the get raw operation.
       */
      getRaw(id) {
        return bridge.messages.getRaw(id)
      },

      /**
       * Request the compatibility report of the message's original HTML.
       */
      async htmlCheck(id) {
        return normalizeCompatibilityReport(await bridge.messages.htmlCheck(id))
      },

      /**
       * Read the message's stored link check without starting one.
       */
      async linkCheck(id) {
        return normalizeLinkCheckReport(
          await nativeCall(() => bridge.messages.linkCheck(id))
        )
      },

      /**
       * Start, or join, a background check of the message's links.
       */
      async startLinkCheck(id) {
        return normalizeLinkCheckReport(
          await nativeCall(() => bridge.messages.linkCheckStart(id))
        )
      },

      /**
       * Start background link checks of the selected messages.
       */
      async checkLinks(identifiers) {
        return normalizeBulkLinkCheck(
          await nativeCall(() => bridge.messages.checkLinks({ identifiers }))
        )
      },

      /**
       * Delete the record identified by the supplied ID.
       */
      delete(id) {
        return bridge.messages.delete(id)
      },

      /**
       * Move the selected messages to the requested folder.
       */
      async move(identifiers, folderIdentifier) {
        return Number(
          (
            await nativeCall(() =>
              bridge.messages.move({ identifiers, folderIdentifier })
            )
          ).moved
        )
      },

      /**
       * Move the selected messages to Trash.
       */
      async trash(identifiers) {
        return Number(
          (await nativeCall(() => bridge.messages.trash({ identifiers })))
            .trashed
        )
      },

      /**
       * Perform the get attachment operation.
       */
      getAttachment(id, index) {
        return bridge.messages.getAttachment(id, index)
      },
    },
    trash: {
      /**
       * Fetch message summaries and convert each wire row to the frontend contract.
       */
      async listMessages(options = {}) {
        return (
          await nativeCall(() =>
            bridge.trash.listMessages({
              before: options.before,
              limit: options.limit,
              query: options.query,
            })
          )
        ).map(normalizeSummary)
      },

      /**
       * Permanently remove messages currently held in Trash and return the removed count.
       */
      async empty() {
        return Number((await nativeCall(() => bridge.trash.empty())).deleted)
      },
    },
    personas: bridge.personas ? personaClient(bridge.personas) : undefined,
    imap: {
      /**
       * Read the global SMTP listener configuration and runtime status.
       */
      server: async () =>
        normalizeImapServer(await nativeCall(() => bridge.imap.server())),

      /**
       * List the records available through this client method.
       */
      list: async () =>
        (await nativeCall(() => bridge.imap.list())).map(normalizeImapAccount),

      /**
       * Retrieve the record identified by the supplied ID.
       */
      get: async (id) =>
        normalizeImapAccount(await nativeCall(() => bridge.imap.get(id))),

      /**
       * Create a record from the supplied input.
       */
      create: async (input) =>
        normalizeIssuedImapCredentials(
          await nativeCall(() => bridge.imap.create(imapAccountWire(input)))
        ),

      /**
       * Apply the supplied changes to the selected record.
       */
      update: async (id, changes) =>
        normalizeImapAccount(
          await nativeCall(() =>
            bridge.imap.update(id, imapAccountUpdateWire(changes))
          )
        ),

      /**
       * Delete the record identified by the supplied ID.
       */
      delete: (id) => nativeCall(() => bridge.imap.delete(id)),

      /**
       * Issue a replacement credential and invalidate the previous password and sessions.
       */
      regeneratePassword: async (id) =>
        normalizeIssuedImapCredentials(
          await nativeCall(() => bridge.imap.regenerate(id))
        ),

      /**
       * Restore messages hidden by mail-client expunges and return the restored count.
       */
      restoreHidden: async (id) =>
        Number((await nativeCall(() => bridge.imap.restore(id))).restored),
      folders: {
        /**
         * Create a record from the supplied input.
         */
        create: async (accountId, input) =>
          normalizeImapFolder(
            await nativeCall(() =>
              bridge.imap.folders.create(accountId, imapFolderWire(input))
            )
          ),

        /**
         * Apply the supplied changes to the selected record.
         */
        update: async (accountId, folderId, input) =>
          normalizeImapFolder(
            await nativeCall(() =>
              bridge.imap.folders.update(
                accountId,
                folderId,
                imapFolderWire(input)
              )
            )
          ),

        /**
         * Delete the record identified by the supplied ID.
         */
        delete: (accountId, folderId) =>
          nativeCall(() => bridge.imap.folders.delete(accountId, folderId)),
      },
    },
    admin: {
      smtp: {
        /**
         * Read the global SMTP listener configuration and runtime status.
         */
        server: async () =>
          normalizeSmtpServer(
            await nativeCall(() => bridge.admin.smtp.server())
          ),

        /**
         * List the records available through this client method.
         */
        list: async () =>
          (await nativeCall(() => bridge.admin.smtp.list())).map(
            normalizeSmtpAccess
          ),

        /**
         * Create a record from the supplied input.
         */
        create: async (input) =>
          normalizeIssuedCredentials(
            await nativeCall(() =>
              bridge.admin.smtp.create(smtpAccessWire(input))
            )
          ),

        /**
         * Issue a replacement credential and invalidate the previous password and sessions.
         */
        regeneratePassword: async (id) =>
          normalizeIssuedCredentials(
            await nativeCall(() => bridge.admin.smtp.regenerate(id))
          ),

        /**
         * Enable or disable the SMTP access.
         */
        setEnabled: async (id, enabled) =>
          normalizeSmtpAccess(
            await nativeCall(() => bridge.admin.smtp.update(id, { enabled }))
          ),

        /**
         * Replace the access authentication rules.
         */
        updateRules: async (id, rules) =>
          normalizeSmtpAccess(
            await nativeCall(() =>
              bridge.admin.smtp.update(id, smtpAccessRulesWire(rules))
            )
          ),

        /**
         * Delete the record identified by the supplied ID.
         */
        delete: (id) => nativeCall(() => bridge.admin.smtp.delete(id)),
      },
      scopes: {
        /**
         * List the records available through this client method.
         */
        list: async () =>
          (await nativeCall(() => bridge.admin.scopes.list())).map(
            normalizeScope
          ),

        /**
         * Save the supplied  definition.
         */
        save: async (input) =>
          normalizeScope(
            await nativeCall(() => bridge.admin.scopes.save(scopeWire(input)))
          ),

        /**
         * Delete the record identified by the supplied ID.
         */
        delete: (id) => nativeCall(() => bridge.admin.scopes.delete(id)),

        /**
         * Preview messages matching the supplied scope definition.
         */
        preview: async (input) =>
          (
            await nativeCall(() =>
              bridge.admin.scopes.preview(scopePreviewWire(input))
            )
          ).map(normalizeSummary),

        /**
         * List user identifiers assigned to the selected scope.
         */
        assignments: (scopeId) =>
          nativeCall(() => bridge.admin.scopes.assignments(scopeId)),

        /**
         * Assign the user to the selected scope.
         */
        assign: (scopeId, userId) =>
          nativeCall(() => bridge.admin.scopes.assign(scopeId, userId)),

        /**
         * Remove the user from the selected scope.
         */
        unassign: (scopeId, userId) =>
          nativeCall(() => bridge.admin.scopes.unassign(scopeId, userId)),
      },
      users: {
        /**
         * List the records available through this client method.
         */
        list: async () =>
          (await nativeCall(() => bridge.admin.users.list())).map(
            normalizeAccount
          ),

        /**
         * Create a record from the supplied input.
         */
        create: async (input) =>
          normalizeAccount(
            await nativeCall(() =>
              bridge.admin.users.create(newAccountWire(input))
            )
          ),

        /**
         * Apply the supplied changes to the selected record.
         */
        update: async (id, changes) =>
          normalizeAccount(
            await nativeCall(() =>
              bridge.admin.users.update(id, accountUpdateWire(changes))
            )
          ),

        /**
         * Delete the record identified by the supplied ID.
         */
        delete: (id) => nativeCall(() => bridge.admin.users.delete(id)),
      },

      /**
       * Validate a query and return any compiler diagnostic.
       */
      validateQuery: async (query) =>
        normalizeValidation(
          await nativeCall(() => bridge.admin.validateQuery(query))
        ),
    },
    events: {
      /**
       * Subscribe to runtime events and return a function that removes the subscription.
       */
      subscribe(listener) {
        return bridge.events.subscribe((event) =>
          listener(normalizeEvent(event))
        )
      },
    },
  }
}

/**
 * Convert preload failures into readable errors without leaking native values to the renderer.
 */
async function nativeCall<T>(operation: () => Promise<T>): Promise<T> {
  try {
    return await operation()
  } catch (error) {
    if (error instanceof Error) throw error
    if (typeof error === 'string') throw new Error(error)
    if (
      error &&
      typeof error === 'object' &&
      'message' in error &&
      typeof error.message === 'string'
    ) {
      throw new Error(error.message)
    }
    throw new Error(
      'The desktop authentication service could not complete the request.'
    )
  }
}

/**
 * Convert a preload event to the shared event contract.
 */
function normalizeEvent(event: WireDesktopEvent): SandpostEvent {
  if (event.type === 'message') {
    return { type: 'message', id: event.id, sequence: event.seq }
  }
  return event
}
