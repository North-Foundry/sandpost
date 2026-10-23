<script setup lang="ts">
import { ref } from 'vue'
import { useI18n } from 'vue-i18n'
import { HistoryIcon, RotateCwIcon } from '@lucide/vue'
import type { OutboundDelivery, OutboundError } from '@/api/contracts'
import { useSandpostClient } from '@/runtime/client'
import { describeError } from '@/utils/errors'
import { formatFullDate, formatStamp } from '@/utils/format'
import { cn } from '@/lib/utils'
import { Alert, AlertDescription } from '@/components/ui/alert'
import { Badge } from '@/components/ui/badge'
import { Button } from '@/components/ui/button'
import { Spinner } from '@/components/ui/spinner'
import { TONE_DOT } from '@/features/personas/format'
import DeliveryDetailsSheet from '@/features/outbound/DeliveryDetailsSheet.vue'
import { canRetryDelivery, deliveryTone, isDeliveryUnfinished, outboundErrorText, recipientTone } from '@/features/outbound/delivery'

/**
 * Outbound deliveries of the open message, shown to Owners: per delivery its routing action,
 * outbound, status, recipients and next retry, with attempt history and Retry for failed ones.
 */
const props = defineProps<{ deliveries: OutboundDelivery[] }>()
const emit = defineEmits<{
  /**
   * Ask the parent to reread the deliveries after a retry.
   */
  (event: 'changed'): void
}>()
const { t } = useI18n()
const client = useSandpostClient()
const openId = ref<string | null>(null)
const retrying = ref<string | null>(null)
const retryError = ref('')

/**
 * Schedule a failed delivery again.
 */
async function retry(delivery: OutboundDelivery): Promise<void> {
  if (retrying.value) return
  retrying.value = delivery.id
  retryError.value = ''
  try {
    await client.admin.deliveries.retry(delivery.id)
    emit('changed')
  } catch (failure) {
    retryError.value = describeError(failure, t('outbound.delivery.retryError'))
  } finally {
    retrying.value = null
  }
}

/**
 * Describe an outbound error with its translated category.
 */
function errorText(failure: OutboundError): string {
  return outboundErrorText(failure, t(`outbound.errorKind.${failure.kind}`))
}
</script>

<template>
  <div class="flex flex-col gap-3">
    <p class="text-xs text-muted-foreground">{{ t('outbound.message.description') }}</p>
    <Alert v-if="retryError" variant="destructive" role="alert"><AlertDescription>{{ retryError }}</AlertDescription></Alert>
    <ul class="flex flex-col gap-3">
      <li v-for="delivery in props.deliveries" :key="delivery.id" class="flex flex-col gap-2 rounded-lg border p-3">
        <div class="flex min-w-0 flex-wrap items-center gap-2">
          <Badge variant="outline" class="gap-1.5 font-normal">
            <span :class="cn('size-1.5 rounded-full', TONE_DOT[deliveryTone(delivery.status)])" aria-hidden="true" />{{ t(`outbound.status.${delivery.status}`) }}
          </Badge>
          <Badge variant="secondary">{{ t(`outbound.action.${delivery.actionType}`) }}</Badge>
          <span class="min-w-0 truncate text-sm">{{ t('outbound.delivery.via', { name: delivery.smtpOutboundName }) }}</span>
          <span class="ml-auto flex shrink-0 gap-1">
            <Button variant="ghost" size="sm" @click="openId = delivery.id"><HistoryIcon data-icon="inline-start" />{{ t('outbound.message.history') }}</Button>
            <Button v-if="canRetryDelivery(delivery)" variant="outline" size="sm" :disabled="Boolean(retrying)" @click="retry(delivery)">
              <Spinner v-if="retrying === delivery.id" data-icon="inline-start" aria-hidden="true" /><RotateCwIcon v-else data-icon="inline-start" />{{ t('outbound.delivery.retry') }}
            </Button>
          </span>
        </div>
        <p class="text-xs text-muted-foreground">
          {{ t('outbound.message.attempts', { count: delivery.attemptCount }, delivery.attemptCount) }}
          <template v-if="delivery.nextAttemptAt && isDeliveryUnfinished(delivery.status)">
            · <time :title="formatFullDate(delivery.nextAttemptAt)">{{ t('outbound.message.nextAttempt', { time: formatStamp(delivery.nextAttemptAt) }) }}</time>
          </template>
          <template v-else-if="delivery.deliveredAt">
            · <time :title="formatFullDate(delivery.deliveredAt)">{{ t('outbound.message.deliveredAt', { time: formatStamp(delivery.deliveredAt) }) }}</time>
          </template>
        </p>
        <p v-if="delivery.lastError && delivery.status !== 'delivered'" class="text-xs break-words text-destructive">{{ errorText(delivery.lastError) }}</p>
        <ul class="flex flex-col gap-1">
          <li v-for="recipient in delivery.recipients" :key="recipient.address" class="flex min-w-0 items-start gap-2 text-xs">
            <span :class="cn('mt-1 size-1.5 shrink-0 rounded-full', TONE_DOT[recipientTone(recipient.status)])" aria-hidden="true" />
            <span class="flex min-w-0 flex-1 flex-col">
              <span class="flex min-w-0 items-center gap-2">
                <span class="truncate font-mono">{{ recipient.address }}</span>
                <span class="ml-auto shrink-0 text-muted-foreground">{{ t(`outbound.recipientStatus.${recipient.status}`) }}</span>
              </span>
              <span v-if="recipient.error" class="break-words text-destructive">{{ errorText(recipient.error) }}</span>
            </span>
          </li>
        </ul>
      </li>
    </ul>
    <DeliveryDetailsSheet :delivery-id="openId" @close="openId = null" @changed="emit('changed')" />
  </div>
</template>
