import { ref, type Ref } from 'vue'
import type { ImapAccount, IssuedImapCredentials, SandpostClient } from '@/api/contracts'
import { describeError } from '@/utils/errors'
import { i18n } from '@/i18n'

/**
 * Confirmed account operations shared by the account list and the account editor.
 */
export interface ImapAccountActions {
  /**
   * Credentials shown once by the credentials dialog; cleared as soon as it closes.
   */
  issued: Ref<IssuedImapCredentials | null>
  regenerated: Ref<boolean>
  regenerateTarget: Ref<ImapAccount | null>
  regenerating: Ref<boolean>
  regenerateError: Ref<string>
  restoreTarget: Ref<ImapAccount | null>
  restoring: Ref<boolean>
  restoreError: Ref<string>

  /**
   * The latest completed restoration, reported inline by the page.
   */
  restoreResult: Ref<{ accountName: string; restored: number } | null>
  deleteTarget: Ref<ImapAccount | null>
  deleting: Ref<boolean>
  deleteError: Ref<string>

  /**
   * Show credentials issued elsewhere, such as by the creation dialog.
   */
  showIssued(credentials: IssuedImapCredentials, regenerated: boolean): void

  /**
   * Open the confirmation for replacing an account password.
   */
  confirmRegenerate(account: ImapAccount): void

  /**
   * Open the confirmation for restoring the account's hidden messages.
   */
  confirmRestore(account: ImapAccount): void

  /**
   * Open the confirmation for deleting the account.
   */
  confirmDelete(account: ImapAccount): void

  /**
   * Replace the password and expose the returned credentials once.
   */
  regenerate(): Promise<void>

  /**
   * Restore hidden messages and retain the outcome for inline display.
   */
  restore(): Promise<void>

  /**
   * Delete the account and notify the owning page after the server succeeds.
   */
  remove(): Promise<void>

  /**
   * Forget the issued password as soon as its dialog closes.
   */
  credentialsClosed(): void
}

/**
 * Report the outcome of restoring hidden messages in one sentence.
 */
export function describeRestoration(restored: number, accountName?: string): string {
  return i18n.global.t(
    restored === 0 ? 'imap.restoration.none' : restored === 1 ? 'imap.restoration.one' : 'imap.restoration.many',
    { count: restored, account: accountName ? i18n.global.t('imap.restoration.inAccount', { account: accountName }) : '' },
  )
}

interface ImapAccountActionCallbacks {
  /**
   * An account changed on the server and should be shown as returned.
   */
  changed(account: ImapAccount): void | Promise<void>

  /**
   * Remove the deleted account from the owning page after the server succeeds.
   */
  deleted(id: string): void | Promise<void>
}

/**
 * Build the confirmation state and operations for regenerating, restoring, and deleting accounts.
 */
export function useImapAccountActions(client: SandpostClient, callbacks: ImapAccountActionCallbacks): ImapAccountActions {
  const issued = ref<IssuedImapCredentials | null>(null)
  const regenerated = ref(false)
  const regenerateTarget = ref<ImapAccount | null>(null)
  const regenerating = ref(false)
  const regenerateError = ref('')
  const restoreTarget = ref<ImapAccount | null>(null)
  const restoring = ref(false)
  const restoreError = ref('')
  const restoreResult = ref<{ accountName: string; restored: number } | null>(null)
  const deleteTarget = ref<ImapAccount | null>(null)
  const deleting = ref(false)
  const deleteError = ref('')

  return {
    issued,
    regenerated,
    regenerateTarget,
    regenerating,
    regenerateError,
    restoreTarget,
    restoring,
    restoreError,
    restoreResult,
    deleteTarget,
    deleting,
    deleteError,

    /**
     * Show newly issued credentials and remember whether they came from regeneration.
     */
    showIssued(credentials, wasRegenerated) {
      regenerated.value = wasRegenerated
      issued.value = credentials
    },

    /**
     * Select the account for password replacement and clear its prior error.
     */
    confirmRegenerate(account) {
      regenerateError.value = ''
      regenerateTarget.value = account
    },

    /**
     * Select the account for restoration and clear its prior error.
     */
    confirmRestore(account) {
      restoreError.value = ''
      restoreTarget.value = account
    },

    /**
     * Select the account for deletion and clear its prior error.
     */
    confirmDelete(account) {
      deleteError.value = ''
      deleteTarget.value = account
    },

    /**
     * Replace the selected password, retain the one-time credentials, and update the owning page.
     */
    async regenerate() {
      const target = regenerateTarget.value
      if (!target || regenerating.value) return
      regenerating.value = true
      regenerateError.value = ''
      try {
        const credentials = await client.imap.regeneratePassword(target.id)
        regenerateTarget.value = null
        regenerated.value = true
        issued.value = credentials
        await callbacks.changed(credentials.account)
      } catch (failure) {
        regenerateError.value = describeError(failure, i18n.global.t('imap.errors.regeneratePassword'))
      } finally {
        regenerating.value = false
      }
    },

    /**
     * Restore hidden messages and retain the count for the owning page to display.
     */
    async restore() {
      const target = restoreTarget.value
      if (!target || restoring.value) return
      restoring.value = true
      restoreError.value = ''
      try {
        const restored = await client.imap.restoreHidden(target.id)
        restoreTarget.value = null
        restoreResult.value = { accountName: target.name, restored }
      } catch (failure) {
        restoreError.value = describeError(failure, i18n.global.t('imap.errors.restoreHidden'))
      } finally {
        restoring.value = false
      }
    },

    /**
     * Delete the selected account and notify the owning page after success.
     */
    async remove() {
      const target = deleteTarget.value
      if (!target || deleting.value) return
      deleting.value = true
      deleteError.value = ''
      try {
        await client.imap.delete(target.id)
        deleteTarget.value = null
        await callbacks.deleted(target.id)
      } catch (failure) {
        deleteError.value = describeError(failure, i18n.global.t('imap.errors.deleteAccount'))
      } finally {
        deleting.value = false
      }
    },

    /**
     * Clear the one-time password after its dialog closes.
     */
    credentialsClosed() {
      issued.value = null
      regenerated.value = false
    },
  }
}
