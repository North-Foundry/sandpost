import { i18n } from '@/i18n'
import { onScopeDispose, shallowRef } from 'vue'
import { copyText } from '@/runtime/clipboard'

/**
 * How long a successful copy keeps its confirmation visible, in milliseconds.
 */
export const COPY_CONFIRMATION_DURATION = 2000

/**
 * Copy text and briefly remember which target succeeded, so the caller can swap its copy icon for
 * a confirmation. A failure clears the confirmation and exposes `failureMessage` through `error`.
 */
export function useCopy<Target extends string = string>(
  failureMessage?: string,
) {
  const copied = shallowRef<Target | null>(null)
  const error = shallowRef('')
  let confirmationTimer: ReturnType<typeof setTimeout> | undefined

  /**
   * Copy `text`, mark `target` as copied for a short time, and report whether the copy succeeded.
   */
  async function copy(text: string, target: Target): Promise<boolean> {
    error.value = ''
    clearTimeout(confirmationTimer)

    try {
      await copyText(text)
    } catch {
      copied.value = null
      error.value = failureMessage ?? i18n.global.t('common.copyFailed')
      return false
    }

    copied.value = target
    confirmationTimer = setTimeout(() => { copied.value = null }, COPY_CONFIRMATION_DURATION)

    return true
  }

  /**
   * Drop any confirmation or error, for example when the copied content changes.
   */
  function reset(): void {
    clearTimeout(confirmationTimer)
    copied.value = null
    error.value = ''
  }

  onScopeDispose(() => clearTimeout(confirmationTimer))

  return { copied, error, copy, reset }
}
