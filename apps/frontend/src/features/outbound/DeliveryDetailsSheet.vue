<script setup lang="ts">
import { onBeforeUnmount, ref, watch } from 'vue'
import { useI18n } from 'vue-i18n'
import { RotateCwIcon } from '@lucide/vue'
import type { OutboundDeliveryDetail, OutboundError } from '@/api/contracts'
import { useSandpostClient } from '@/runtime/client'
import { describeError } from '@/utils/errors'
import { formatFullDate, formatStamp } from '@/utils/format'
import { cn } from '@/lib/utils'
import { Alert, AlertDescription } from '@/components/ui/alert'
import { Badge } from '@/components/ui/badge'
import { Button } from '@/components/ui/button'
import { Separator } from '@/components/ui/separator'
import { Sheet, SheetContent, SheetDescription, SheetHeader, SheetTitle } from '@/components/ui/sheet'
import { Skeleton } from '@/components/ui/skeleton'
import { Spinner } from '@/components/ui/spinner'
import { TONE_DOT } from '@/features/personas/format'
import { attemptTone, canRetryDelivery, deliveryTone, isDeliveryUnfinished, outboundErrorText, recipientTone } from './delivery'

/**
 * Side sheet with one delivery's recipients and attempt history, refreshed while it is unfinished.
 * Failed deliveries can be retried from here.
 */
const props = defineProps<{
  /**
   * The delivery to show; null closes the sheet.
   */
  deliveryId: string | null
}>()
const emit = defineEmits<{
  /**
   * Ask the parent to close the sheet.
   */
  (event: 'close'): void

  /**
   * Report that a retry changed the delivery.
   */
  (event: 'changed'): void
}>()
const { t } = useI18n()
const client = useSandpostClient()
const REFRESH_INTERVAL = 3000

const detail = ref<OutboundDeliveryDetail | null>(null)
const loading = ref(false)
const error = ref('')
const retrying = ref(false)
const retryError = ref('')
let generation = 0
let refreshTimer: ReturnType<typeof setTimeout> | null = null

/**
 * Stop the pending refresh.
 */
function stopRefresh(): void {
  if (refreshTimer) clearTimeout(refreshTimer)
  refreshTimer = null
}

/**
 * Load the delivery, ignoring responses for a delivery that is no longer shown.
 */
async function load(): Promise<void> {
  const id = props.deliveryId
  stopRefresh()
  if (!id) return
  const request = ++generation
  if (detail.value?.id !== id) loading.value = true
  try {
    const loaded = await client.admin.deliveries.get(id)
    if (request !== generation) return
    detail.value = loaded
    error.value = ''
    if (isDeliveryUnfinished(loaded.status)) refreshTimer = setTimeout(() => { void load() }, REFRESH_INTERVAL)
  } catch (failure) {
    if (request === generation) error.value = describeError(failure, t('outbound.delivery.loadError'))
  } finally {
    if (request === generation) loading.value = false
  }
}

/**
 * Schedule the failed delivery again and follow its progress.
 */
async function retry(): Promise<void> {
  const current = detail.value
  if (!current || retrying.value) return
  retrying.value = true
  retryError.value = ''
  try {
    await client.admin.deliveries.retry(current.id)
    emit('changed')
    await load()
  } catch (failure) {
    retryError.value = describeError(failure, t('outbound.delivery.retryError'))
  } finally {
    retrying.value = false
  }
}

/**
 * Describe an outbound error with its translated category.
 */
function errorText(failure: OutboundError): string {
  return outboundErrorText(failure, t(`outbound.errorKind.${failure.kind}`))
}

watch(() => props.deliveryId, (id) => {
  generation += 1
  stopRefresh()
  retryError.value = ''
  error.value = ''
  if (!id) { detail.value = null; return }
  void load()
}, { immediate: true })

onBeforeUnmount(() => { generation += 1; stopRefresh() })
</script>

<template>
  <Sheet :open="Boolean(props.deliveryId)" @update:open="!$event && emit('close')">
    <SheetContent side="right" class="flex w-full flex-col gap-0 p-0 sm:max-w-lg">
      <SheetHeader class="border-b px-4 py-3">
        <SheetTitle class="truncate">{{ detail?.subject || t('outbound.delivery.noSubject') }}</SheetTitle>
        <SheetDescription>{{ t('outbound.delivery.description') }}</SheetDescription>
      </SheetHeader>

      <div class="min-h-0 flex-1 overflow-y-auto">
        <Alert v-if="error" variant="destructive" class="m-4 w-auto" role="alert">
          <AlertDescription class="flex items-center gap-3"><span class="flex-1">{{ error }}</span><Button variant="outline" size="sm" @click="load">{{ t('outbound.common.retry') }}</Button></AlertDescription>
        </Alert>
        <div v-if="loading && !detail" class="flex flex-col gap-3 p-4">
          <Skeleton class="h-16 w-full" />
          <Skeleton v-for="index in 3" :key="index" class="h-10 w-full" />
        </div>
        <template v-else-if="detail">
          <section class="flex flex-col gap-3 px-4 py-3">
            <div class="flex flex-wrap items-center gap-2">
              <Badge variant="outline" class="gap-1.5 font-normal">
                <span :class="cn('size-1.5 rounded-full', TONE_DOT[deliveryTone(detail.status)])" aria-hidden="true" />{{ t(`outbound.status.${detail.status}`) }}
              </Badge>
              <Badge variant="secondary">{{ t(`outbound.action.${detail.actionType}`) }}</Badge>
              <span class="truncate text-sm text-muted-foreground">{{ t('outbound.delivery.via', { name: detail.smtpOutboundName }) }}</span>
              <Button v-if="canRetryDelivery(detail)" variant="outline" size="sm" class="ml-auto" :disabled="retrying" @click="retry">
                <Spinner v-if="retrying" data-icon="inline-start" aria-hidden="true" /><RotateCwIcon v-else data-icon="inline-start" />{{ t('outbound.delivery.retry') }}
              </Button>
            </div>
            <dl class="grid grid-cols-[auto_minmax(0,1fr)] gap-x-3 gap-y-1 text-xs">
              <dt class="text-muted-foreground">{{ t('outbound.delivery.created') }}</dt>
              <dd>{{ formatFullDate(detail.createdAt) }}</dd>
              <template v-if="detail.envelopeFrom">
                <dt class="text-muted-foreground">{{ t('outbound.delivery.envelopeFrom') }}</dt>
                <dd class="truncate font-mono">{{ detail.envelopeFrom }}</dd>
              </template>
              <dt class="text-muted-foreground">{{ t('outbound.delivery.attempts') }}</dt>
              <dd class="tabular-nums">{{ detail.attemptCount }}</dd>
              <template v-if="detail.nextAttemptAt && isDeliveryUnfinished(detail.status)">
                <dt class="text-muted-foreground">{{ t('outbound.delivery.nextAttempt') }}</dt>
                <dd>{{ formatFullDate(detail.nextAttemptAt) }}</dd>
              </template>
              <template v-if="detail.deliveredAt">
                <dt class="text-muted-foreground">{{ t('outbound.delivery.deliveredAt') }}</dt>
                <dd>{{ formatFullDate(detail.deliveredAt) }}</dd>
              </template>
            </dl>
            <p v-if="detail.lastError" class="text-xs break-words text-destructive">{{ errorText(detail.lastError) }}</p>
            <Alert v-if="retryError" variant="destructive" role="alert"><AlertDescription>{{ retryError }}</AlertDescription></Alert>
          </section>
          <Separator />
          <section class="flex flex-col gap-2 px-4 py-3" :aria-label="t('outbound.delivery.recipients')">
            <h3 class="text-xs font-medium text-muted-foreground">{{ t('outbound.delivery.recipients') }}</h3>
            <ul class="flex flex-col">
              <li v-for="recipient in detail.recipients" :key="recipient.address" class="flex min-w-0 items-start gap-2 border-b border-border/60 py-2 text-xs last:border-b-0">
                <span :class="cn('mt-1 size-1.5 shrink-0 rounded-full', TONE_DOT[recipientTone(recipient.status)])" aria-hidden="true" />
                <span class="flex min-w-0 flex-1 flex-col gap-0.5">
                  <span class="flex min-w-0 items-center gap-2">
                    <span class="truncate font-mono">{{ recipient.address }}</span>
                    <span class="ml-auto shrink-0 text-muted-foreground">{{ t(`outbound.recipientStatus.${recipient.status}`) }}</span>
                  </span>
                  <span v-if="recipient.error" class="break-words text-destructive">{{ errorText(recipient.error) }}</span>
                  <span v-else-if="recipient.response" class="truncate font-mono text-muted-foreground" :title="recipient.response">{{ recipient.response }}</span>
                </span>
              </li>
            </ul>
          </section>
          <Separator />
          <section class="flex flex-col gap-2 px-4 py-3" :aria-label="t('outbound.delivery.history')">
            <h3 class="text-xs font-medium text-muted-foreground">{{ t('outbound.delivery.history') }}</h3>
            <p v-if="!detail.attempts.length" class="text-xs text-muted-foreground">{{ t('outbound.delivery.noAttempts') }}</p>
            <ol class="flex flex-col">
              <li v-for="attempt in detail.attempts" :key="attempt.number" class="flex min-w-0 items-start gap-2 border-b border-border/60 py-2 text-xs last:border-b-0">
                <span :class="cn('mt-1 size-1.5 shrink-0 rounded-full', TONE_DOT[attemptTone(attempt.outcome)])" aria-hidden="true" />
                <span class="flex min-w-0 flex-1 flex-col gap-0.5">
                  <span class="flex min-w-0 items-center gap-2">
                    <span class="font-medium">{{ t('outbound.delivery.attempt', { number: attempt.number }) }}</span>
                    <span class="text-muted-foreground">{{ t(`outbound.outcome.${attempt.outcome}`) }}</span>
                    <time class="ml-auto shrink-0 text-muted-foreground" :title="formatFullDate(attempt.startedAt)">{{ formatStamp(attempt.startedAt) }}</time>
                  </span>
                  <span class="text-muted-foreground">
                    {{ t('outbound.delivery.attemptCounts', { delivered: attempt.deliveredRecipients, deferred: attempt.deferredRecipients, failed: attempt.failedRecipients }) }}
                  </span>
                  <span v-if="attempt.error" class="break-words text-destructive">{{ errorText(attempt.error) }}</span>
                </span>
              </li>
            </ol>
          </section>
        </template>
      </div>
    </SheetContent>
  </Sheet>
</template>
