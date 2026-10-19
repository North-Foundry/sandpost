import type {
  LoginInput,
  MessageFlagOperation,
  MessageFlags,
  MessageListOptions,
  SessionUser,
  SetupInput,
} from './contracts'
import type {
  WireAccount,
  WireImapAccount,
  WireImapAccountInput,
  WireImapAccountUpdate,
  WireImapFolderInput,
  WireImapLocalFolder,
  WireImapServer,
  WireInbox,
  WireIssuedImapCredentials,
  WireIssuedSmtpCredentials,
  WireCompatibilityReport,
  WireMessageDetail,
  WireMessageSummary,
  WireQueryValidation,
  WireScope,
  WireSmtpAccess,
  WireSmtpAccessInput,
  WireSmtpAccessUpdate,
  WireSmtpServer,
  WireView,
  WireWebFolder,
} from './wire'

/**
 * Events delivered by the isolated Electron preload bridge.
 */
export type WireDesktopEvent =
  | { type: 'ready' | 'resync' | 'disconnected' }
  | { type: 'message'; id: string; seq: number }

/**
 * Narrow renderer API exposed by Electron's context-isolated preload.
 */
export interface SandpostDesktopBridge {
  runtime: 'electron'

  /**
   * `inset` when native window controls are drawn over the top-left of the renderer.
   */
  titleBar?: 'inset' | 'native'
  preview?: SandpostEmailPreviewBridge

  /**
   * Read whether initial setup is complete and whether a setup key is required.
   */
  instanceStatus(): Promise<{ initialized: boolean; setupKeyRequired: false }>

  /**
   * Create the initial owner using the temporary bootstrap authorization.
   */
  setup(input: SetupInput): Promise<void>

  /**
   * Authenticate the supplied credentials and return the signed-in user.
   */
  login(input: LoginInput): Promise<SessionUser>

  /**
   * Read the current session, returning null when it is no longer authenticated.
   */
  session(): Promise<SessionUser | null>

  /**
   * End the current authenticated session.
   */
  logout(): Promise<void>
  inboxes: {
    /**
     * List the records available through this client method.
     */
    list(): Promise<WireInbox[]>

    /**
     * Create a record from the supplied input.
     */
    create(name: string, address: string): Promise<WireInbox>
  }
  folders: {
    /**
     * List the records available through this client method.
     */
    list(): Promise<WireWebFolder[]>

    /**
     * Create a record from the supplied input.
     */
    create(input: {
      name: string
      icon?: string | null
      exposedAccounts?: string[]
    }): Promise<WireWebFolder>

    /**
     * Apply the supplied changes to the selected record.
     */
    update(
      id: string,
      input: { name: string; icon?: string | null; exposedAccounts?: string[] }
    ): Promise<WireWebFolder>

    /**
     * Delete the record identified by the supplied ID.
     */
    delete(id: string): Promise<void>

    /**
     * Fetch message summaries and convert each wire row to the frontend contract.
     */
    listMessages(
      id: string,
      options?: { before?: number; limit?: number; query?: string }
    ): Promise<WireMessageSummary[]>
  }
  views: {
    /**
     * List the records available through this client method.
     */
    list(): Promise<WireView[]>

    /**
     * Save the supplied  definition.
     */
    save(input: WireView): Promise<WireView>

    /**
     * Delete the record identified by the supplied ID.
     */
    delete(id: string): Promise<void>
  }
  messages: {
    /**
     * List the records available through this client method.
     */
    list(options?: MessageListOptions): Promise<WireMessageSummary[]>

    /**
     * Search messages and return matching summaries.
     */
    search(
      query: string,
      before?: number,
      limit?: number
    ): Promise<WireMessageSummary[]>

    /**
     * Set the Seen flag for the selected messages and return the number changed.
     */
    setSeen(input: {
      identifiers: string[]
      seen: boolean
    }): Promise<{ changed: number }>

    /**
     * Copy the selected messages to the requested folder.
     */
    copy(input: {
      identifiers: string[]
      folderIdentifier: string | null
    }): Promise<{ copied: number }>

    /**
     * Apply the requested flag operation to the selected messages.
     */
    setFlags(input: {
      identifiers: string[]
      operation: MessageFlagOperation
      flags: MessageFlags
    }): Promise<{ changed: number }>

    /**
     * Retrieve the record identified by the supplied ID.
     */
    get(id: string): Promise<WireMessageDetail>

    /**
     * Perform the get raw operation.
     */
    getRaw(id: string): Promise<Uint8Array>

    /**
     * Check the message's original HTML against email client support data.
     */
    htmlCheck(id: string): Promise<WireCompatibilityReport>

    /**
     * Delete the record identified by the supplied ID.
     */
    delete(id: string): Promise<void>

    /**
     * Move the selected messages to the requested folder.
     */
    move(input: {
      identifiers: string[]
      folderIdentifier: string | null
    }): Promise<{ moved: number }>

    /**
     * Move the selected messages to Trash.
     */
    trash(input: { identifiers: string[] }): Promise<{ trashed: number }>

    /**
     * Perform the get attachment operation.
     */
    getAttachment(id: string, index: number): Promise<Uint8Array>
  }
  trash: {
    /**
     * Fetch message summaries and convert each wire row to the frontend contract.
     */
    listMessages(options?: {
      before?: number
      limit?: number
      query?: string
    }): Promise<WireMessageSummary[]>

    /**
     * Permanently remove messages currently held in Trash and return the removed count.
     */
    empty(): Promise<{ deleted: number }>
  }
  clipboard: {
    /**
     * Write the supplied text to the system clipboard.
     */
    writeText(text: string): Promise<void>
  }
  events: {
    /**
     * Subscribe to runtime events and return a function that removes the subscription.
     */
    subscribe(listener: (event: WireDesktopEvent) => void): Promise<() => void>
  }
  imap: SandpostDesktopImapBridge
  admin: SandpostDesktopAdminBridge
}

/**
 * Native preview capability; coordinates are CSS pixels relative to the app viewport.
 */
export interface SandpostEmailPreviewBridge {
  /**
   * Display the supplied email in the native preview.
   */
  show(input: { id: string; html: string }): Promise<void>

  /**
   * Update native preview rectangles and the active theme.
   */
  layout(input: {
    id: string
    preview: EmailPreviewRectangle | null
    devtools: EmailPreviewRectangle | null
    theme: 'light' | 'dark'
  }): Promise<void>

  /**
   * Set whether the native inspector is open for the email preview.
   */
  inspect(input: { id: string; open: boolean }): Promise<void>

  /**
   * Release native preview resources for the email identifier.
   */
  release(id: string): Promise<void>

  /**
   * Subscribe to runtime events and return a function that removes the subscription.
   */
  subscribe(
    listener: (state: {
      id: string
      open: boolean
      ready?: boolean
      error?: string
    }) => void
  ): () => void
}

export interface EmailPreviewRectangle {
  x: number
  y: number
  width: number
  height: number
}

/**
 * The signed-in user's IMAP accounts, each operation on its own validated IPC channel.
 */
export interface SandpostDesktopImapBridge {
  /**
   * Read the IMAP server connection details.
   */
  server(): Promise<WireImapServer>

  /**
   * List the records available through this client method.
   */
  list(): Promise<WireImapAccount[]>

  /**
   * Retrieve the record identified by the supplied ID.
   */
  get(id: string): Promise<WireImapAccount>

  /**
   * Create a record from the supplied input.
   */
  create(input: WireImapAccountInput): Promise<WireIssuedImapCredentials>

  /**
   * Apply the supplied changes to the selected record.
   */
  update(id: string, input: WireImapAccountUpdate): Promise<WireImapAccount>

  /**
   * Delete the record identified by the supplied ID.
   */
  delete(id: string): Promise<void>

  /**
   * Issue a replacement credential and invalidate the previous password.
   */
  regenerate(id: string): Promise<WireIssuedImapCredentials>

  /**
   * Restore messages hidden by mail-client expunges and return the restored count.
   */
  restore(id: string): Promise<{ restored: number }>
  folders: {
    /**
     * Create a record from the supplied input.
     */
    create(
      accountId: string,
      input: WireImapFolderInput
    ): Promise<WireImapLocalFolder>

    /**
     * Apply the supplied changes to the selected record.
     */
    update(
      accountId: string,
      folderId: string,
      input: WireImapFolderInput
    ): Promise<WireImapLocalFolder>

    /**
     * Delete the record identified by the supplied ID.
     */
    delete(accountId: string, folderId: string): Promise<void>
  }
}

/**
 * Administration operations, each forwarded over its own validated IPC channel.
 */
export interface SandpostDesktopAdminBridge {
  smtp: {
    /**
     * Read the IMAP server connection details.
     */
    server(): Promise<WireSmtpServer>

    /**
     * List the records available through this client method.
     */
    list(): Promise<WireSmtpAccess[]>

    /**
     * Create a record from the supplied input.
     */
    create(input: WireSmtpAccessInput): Promise<WireIssuedSmtpCredentials>

    /**
     * Issue a replacement credential and invalidate the previous password.
     */
    regenerate(id: string): Promise<WireIssuedSmtpCredentials>

    /**
     * Apply the supplied changes to the selected record.
     */
    update(id: string, input: WireSmtpAccessUpdate): Promise<WireSmtpAccess>

    /**
     * Delete the record identified by the supplied ID.
     */
    delete(id: string): Promise<void>
  }
  scopes: {
    /**
     * List the records available through this client method.
     */
    list(): Promise<WireScope[]>

    /**
     * Save the supplied  definition.
     */
    save(input: WireScope): Promise<WireScope>

    /**
     * Delete the record identified by the supplied ID.
     */
    delete(id: string): Promise<void>

    /**
     * Preview messages matching the supplied scope definition.
     */
    preview(input: {
      parent: string | null
      filter: string
      limit: number
    }): Promise<WireMessageSummary[]>

    /**
     * List user identifiers assigned to the selected scope.
     */
    assignments(scopeId: string): Promise<string[]>

    /**
     * Assign the user to the selected scope.
     */
    assign(scopeId: string, userId: string): Promise<void>

    /**
     * Remove the user from the selected scope.
     */
    unassign(scopeId: string, userId: string): Promise<void>
  }
  users: {
    /**
     * List the records available through this client method.
     */
    list(): Promise<WireAccount[]>

    /**
     * Create a record from the supplied input.
     */
    create(input: {
      name: string
      email: string
      password: string
      role: string
      mail_access: string
    }): Promise<WireAccount>

    /**
     * Apply the supplied changes to the selected record.
     */
    update(
      id: string,
      input: {
        name?: string
        password?: string
        role?: string
        mail_access?: string
      }
    ): Promise<WireAccount>

    /**
     * Delete the record identified by the supplied ID.
     */
    delete(id: string): Promise<void>
  }

  /**
   * Validate a query and return any compiler diagnostic.
   */
  validateQuery(query: string): Promise<WireQueryValidation>
}

/**
 * Check the preload marker and callable bridge surface before selecting it.
 */
export function isSandpostDesktopBridge(
  value: unknown
): value is SandpostDesktopBridge {
  if (!value || typeof value !== 'object') return false
  const bridge = value as Partial<SandpostDesktopBridge>
  return bridge.runtime === 'electron' &&
    typeof bridge.instanceStatus === 'function' &&
    typeof bridge.setup === 'function' &&
    typeof bridge.login === 'function' &&
    typeof bridge.session === 'function' &&
    typeof bridge.logout === 'function' &&
    typeof bridge.inboxes?.list === 'function' &&
    typeof bridge.inboxes.create === 'function' &&
    typeof bridge.views?.list === 'function' &&
    typeof bridge.views.save === 'function' &&
    typeof bridge.views.delete === 'function' &&
    typeof bridge.folders?.list === 'function' &&
    typeof bridge.folders.create === 'function' &&
    typeof bridge.folders.update === 'function' &&
    typeof bridge.folders.delete === 'function' &&
    typeof bridge.folders.listMessages === 'function' &&
    typeof bridge.messages?.list === 'function' &&
    typeof bridge.messages.search === 'function' &&
    typeof bridge.messages.setSeen === 'function' &&
    typeof bridge.messages.copy === 'function' &&
    typeof bridge.messages.setFlags === 'function' &&
    typeof bridge.messages.get === 'function' &&
    typeof bridge.messages.getRaw === 'function' &&
    typeof bridge.messages.htmlCheck === 'function' &&
    typeof bridge.messages.delete === 'function' &&
    typeof bridge.messages.move === 'function' &&
    typeof bridge.messages.trash === 'function' &&
    typeof bridge.trash?.listMessages === 'function' &&
    typeof bridge.trash.empty === 'function' &&
    typeof bridge.messages.getAttachment === 'function' &&
    typeof bridge.clipboard?.writeText === 'function' &&
    typeof bridge.events?.subscribe === 'function' &&
    typeof bridge.imap?.list === 'function' &&
    typeof bridge.imap.folders?.create === 'function' &&
    typeof bridge.admin?.smtp?.list === 'function' &&
    typeof bridge.admin.scopes?.preview === 'function' &&
    typeof bridge.admin.users?.list === 'function' &&
    typeof bridge.admin.validateQuery === 'function'

}

declare global {
  interface Window {
    sandpost?: SandpostDesktopBridge
  }
}
