import { onScopeDispose, ref, shallowRef, watch, type Ref } from 'vue'
import type { OutboundDelivery, SandpostClient } from '@/api/contracts'
import { isDeliveryUnfinished } from '@/features/outbound/delivery'

/**
 * How often unfinished deliveries of the open message are reread.
 */
export const MESSAGE_DELIVERY_POLL_INTERVAL = 5000

/**
 * Follow the outbound deliveries of the open message, for Owners only.
 *
 * Nothing is requested while `enabled` is false, so Members and Viewers never call the Owner-only
 * endpoint. Failures stay silent and leave the list empty: delivery status is supplementary and must
 * not disturb reading the message. Unfinished deliveries are polled; responses for a message that is
 * no longer open are ignored.
 */
export function useMessageDeliveries(client: SandpostClient, messageId: Ref<string | null | undefined>, enabled: Ref<boolean>) {
  const deliveries = shallowRef<OutboundDelivery[]>([])
  const loading = ref(false)
  let generation = 0
  let pollTimer: ReturnType<typeof setTimeout> | undefined

  /**
   * Stop the pending poll.
   */
  function stopPolling(): void {
    if (pollTimer !== undefined) clearTimeout(pollTimer)
    pollTimer = undefined
  }

  /**
   * Read the open message's deliveries and keep polling while any can still change.
   */
  async function refresh(current = generation): Promise<void> {
    const id = messageId.value
    stopPolling()
    if (!id || !enabled.value || current !== generation) return
    loading.value = true
    try {
      const loaded = await client.admin.deliveries.forMessage(id)
      if (current !== generation) return
      deliveries.value = loaded
      if (loaded.some((delivery) => isDeliveryUnfinished(delivery.status))) {
        pollTimer = setTimeout(() => { void refresh(current) }, MESSAGE_DELIVERY_POLL_INTERVAL)
      }
    } catch {
      /*
       * Keep the last known list; delivery status never interrupts reading mail.
       */
    } finally {
      if (current === generation) loading.value = false
    }
  }

  watch([messageId, enabled], () => {
    generation += 1
    stopPolling()
    deliveries.value = []
    loading.value = false
    void refresh(generation)
  }, { immediate: true })

  onScopeDispose(() => {
    generation += 1
    stopPolling()
  })

  return {
    deliveries,
    loading,

    /**
     * Reread the open message's deliveries now.
     */
    refresh: () => refresh(generation),
  }
}
