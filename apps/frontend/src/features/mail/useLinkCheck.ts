import { computed, onScopeDispose, ref, shallowRef, watch, type Ref } from 'vue'
import { useI18n } from 'vue-i18n'
import type { LinkCheckReport, SandpostClient } from '@/api/contracts'
import { describeError } from '@/utils/errors'
import { resultsByUrl } from './linkCheck'

/**
 * How often a running check is polled until it completes.
 */
export const LINK_CHECK_POLL_INTERVAL = 1000

/**
 * How often relative times are refreshed while a message is open.
 */
const CLOCK_INTERVAL = 30_000

/**
 * Load and follow the shared link check of the open message.
 *
 * Opening a message only reads stored results; a check starts solely through `start`. While a check
 * runs, whoever triggered it, the state is polled and the previous results stay visible until the
 * replacement arrives. Responses for a message that is no longer open are ignored.
 */
export function useLinkCheck(client: SandpostClient, messageId: Ref<string | null | undefined>) {
  const { t } = useI18n()
  const report = shallowRef<LinkCheckReport | null>(null)
  const loading = ref(false)
  const starting = ref(false)
  const error = ref('')
  const now = ref(Date.now())
  let generation = 0
  let pollTimer: ReturnType<typeof setTimeout> | undefined
  const clockTimer = setInterval(() => { now.value = Date.now() }, CLOCK_INTERVAL)

  /**
   * Stop polling a previous check.
   */
  function stopPolling(): void {
    if (pollTimer !== undefined) clearTimeout(pollTimer)
    pollTimer = undefined
  }

  /**
   * Accept a report for the open message and keep polling while its check runs.
   */
  function accept(next: LinkCheckReport, current: number): void {
    if (current !== generation) return
    report.value = next
    now.value = Date.now()
    stopPolling()
    if (next.running) pollTimer = setTimeout(() => { void refresh(current) }, LINK_CHECK_POLL_INTERVAL)
  }

  /**
   * Reread the open message's state; a failed poll keeps the last known results and retries.
   */
  async function refresh(current = generation): Promise<void> {
    const id = messageId.value
    if (!id || current !== generation) return
    try {
      accept(await client.messages.linkCheck(id), current)
      if (current === generation) error.value = ''
    } catch (failure) {
      if (current !== generation) return
      error.value = describeError(failure, t('mail.links.loadFailed'))
      if (report.value?.running) pollTimer = setTimeout(() => { void refresh(current) }, LINK_CHECK_POLL_INTERVAL)
    }
  }

  /**
   * Start, or join, a check of the open message's links.
   */
  async function start(): Promise<void> {
    const id = messageId.value
    if (!id || starting.value) return
    const current = generation
    starting.value = true
    error.value = ''
    try {
      accept(await client.messages.startLinkCheck(id), current)
    } catch (failure) {
      if (current === generation) error.value = describeError(failure, t('mail.links.startFailed'))
    } finally {
      if (current === generation) starting.value = false
    }
  }

  watch(messageId, async (id) => {
    const current = ++generation
    stopPolling()
    report.value = null
    error.value = ''
    starting.value = false
    if (!id) return
    loading.value = true
    try {
      await refresh(current)
    } finally {
      if (current === generation) loading.value = false
    }
  }, { immediate: true })

  onScopeDispose(() => {
    generation += 1
    stopPolling()
    clearInterval(clockTimer)
  })

  return {
    report,
    loading,
    starting,
    error,
    now,
    running: computed(() => starting.value || Boolean(report.value?.running)),
    results: computed(() => resultsByUrl(report.value)),
    start,
    refresh: () => refresh(),
  }
}

export type LinkCheckState = ReturnType<typeof useLinkCheck>
