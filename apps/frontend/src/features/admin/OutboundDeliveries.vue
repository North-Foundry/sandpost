<script setup lang="ts">
import { onMounted, ref, watch } from 'vue'
import { useI18n } from 'vue-i18n'
import { RefreshCwIcon, SendIcon } from '@lucide/vue'
import type { OutboundDelivery, OutboundDeliveryStatus } from '@/api/contracts'
import { useSandpostClient } from '@/runtime/client'
import { describeError } from '@/utils/errors'
import { formatFullDate, formatListDate } from '@/utils/format'
import { cn } from '@/lib/utils'
import { Alert, AlertDescription } from '@/components/ui/alert'
import { Badge } from '@/components/ui/badge'
import { Button } from '@/components/ui/button'
import { Card, CardAction, CardContent, CardDescription, CardHeader, CardTitle } from '@/components/ui/card'
import { Empty, EmptyDescription, EmptyHeader, EmptyMedia, EmptyTitle } from '@/components/ui/empty'
import { NativeSelect, NativeSelectOption } from '@/components/ui/native-select'
import { Skeleton } from '@/components/ui/skeleton'
import { Spinner } from '@/components/ui/spinner'
import { TONE_DOT } from '@/features/personas/format'
import DeliveryDetailsSheet from '@/features/outbound/DeliveryDetailsSheet.vue'
import { DELIVERY_STATUSES, deliveryTone, recipientCounts } from '@/features/outbound/delivery'

/**
 * Compact list of recent outbound deliveries with a status filter, keyset paging and details.
 */
const { t } = useI18n()
const client = useSandpostClient()
const PAGE_SIZE = 25
const ALL = ''

const deliveries = ref<OutboundDelivery[]>([])
const status = ref<OutboundDeliveryStatus | typeof ALL>(ALL)
const loading = ref(true)
const loadingOlder = ref(false)
const hasOlder = ref(false)
const error = ref('')
const openId = ref<string | null>(null)
let generation = 0

/**
 * Load the newest page for the current filter, ignoring superseded responses.
 */
async function load(): Promise<void> {
  const request = ++generation
  loading.value = true
  error.value = ''
  try {
    const page = await client.admin.deliveries.list({ status: status.value || undefined, limit: PAGE_SIZE })
    if (request !== generation) return
    deliveries.value = page
    hasOlder.value = page.length === PAGE_SIZE
  } catch (failure) {
    if (request === generation) error.value = describeError(failure, t('outbound.deliveries.loadError'))
  } finally {
    if (request === generation) loading.value = false
  }
}

/**
 * Append the next older page after the last loaded delivery.
 */
async function loadOlder(): Promise<void> {
  const last = deliveries.value.at(-1)
  if (!last || loadingOlder.value) return
  const request = generation
  loadingOlder.value = true
  try {
    const page = await client.admin.deliveries.list({ status: status.value || undefined, before: last.sequence, limit: PAGE_SIZE })
    if (request !== generation) return
    deliveries.value = [...deliveries.value, ...page]
    hasOlder.value = page.length === PAGE_SIZE
  } catch (failure) {
    if (request === generation) error.value = describeError(failure, t('outbound.deliveries.loadError'))
  } finally {
    loadingOlder.value = false
  }
}

/**
 * Summarize how many recipients were delivered out of the total.
 */
function recipientSummary(delivery: OutboundDelivery): string {
  const counts = recipientCounts(delivery)
  return t('outbound.deliveries.recipientSummary', { delivered: counts.delivered, total: delivery.recipients.length })
}

watch(status, () => { void load() })
onMounted(load)
defineExpose({ load })
</script>

<template>
  <Card>
    <CardHeader>
      <CardTitle>{{ t('outbound.deliveries.title') }}</CardTitle>
      <CardDescription>{{ t('outbound.deliveries.description') }}</CardDescription>
      <CardAction class="flex items-center gap-2">
        <NativeSelect v-model="status" size="sm" :aria-label="t('outbound.deliveries.filter')">
          <NativeSelectOption :value="ALL">{{ t('outbound.deliveries.allStatuses') }}</NativeSelectOption>
          <NativeSelectOption v-for="candidate in DELIVERY_STATUSES" :key="candidate" :value="candidate">{{ t(`outbound.status.${candidate}`) }}</NativeSelectOption>
        </NativeSelect>
        <Button variant="ghost" size="icon-sm" :aria-label="t('outbound.deliveries.refresh')" :disabled="loading" @click="load">
          <Spinner v-if="loading && deliveries.length" /><RefreshCwIcon v-else />
        </Button>
      </CardAction>
    </CardHeader>
    <CardContent class="flex flex-col gap-3">
      <Alert v-if="error" variant="destructive" role="alert">
        <AlertDescription class="flex items-center gap-3"><span class="flex-1">{{ error }}</span><Button variant="outline" size="sm" @click="load">{{ t('outbound.common.retry') }}</Button></AlertDescription>
      </Alert>
      <div v-if="loading && !deliveries.length" class="flex flex-col gap-2">
        <Skeleton v-for="index in 3" :key="index" class="h-10 w-full" />
      </div>
      <Empty v-else-if="!deliveries.length && !error" class="border py-8">
        <EmptyHeader>
          <EmptyMedia variant="icon"><SendIcon /></EmptyMedia>
          <EmptyTitle>{{ t('outbound.deliveries.empty') }}</EmptyTitle>
          <EmptyDescription>{{ t('outbound.deliveries.emptyDescription') }}</EmptyDescription>
        </EmptyHeader>
      </Empty>
      <ul v-else class="flex flex-col overflow-hidden rounded-lg border">
        <li v-for="delivery in deliveries" :key="delivery.id" class="border-b last:border-b-0">
          <button
            type="button"
            class="flex w-full min-w-0 items-center gap-3 px-3 py-2 text-left outline-none hover:bg-accent focus-visible:ring-3 focus-visible:ring-ring/50"
            @click="openId = delivery.id"
          >
            <span :class="cn('size-2 shrink-0 rounded-full', TONE_DOT[deliveryTone(delivery.status)])" aria-hidden="true" />
            <span class="flex min-w-0 flex-1 flex-col">
              <span class="truncate text-sm font-medium">{{ delivery.subject || t('outbound.delivery.noSubject') }}</span>
              <span class="truncate text-xs text-muted-foreground">
                {{ t(`outbound.status.${delivery.status}`) }} · {{ t(`outbound.action.${delivery.actionType}`) }} · {{ delivery.smtpOutboundName }} · {{ recipientSummary(delivery) }}
              </span>
            </span>
            <Badge v-if="delivery.attemptCount > 1" variant="outline" class="shrink-0 tabular-nums max-sm:hidden">{{ t('outbound.deliveries.attemptCount', { count: delivery.attemptCount }) }}</Badge>
            <time class="shrink-0 text-xs text-muted-foreground" :title="formatFullDate(delivery.createdAt)">{{ formatListDate(delivery.createdAt) }}</time>
          </button>
        </li>
      </ul>
      <Button v-if="hasOlder && deliveries.length" variant="outline" size="sm" class="self-center" :disabled="loadingOlder" @click="loadOlder">
        <Spinner v-if="loadingOlder" data-icon="inline-start" aria-hidden="true" />{{ loadingOlder ? t('outbound.deliveries.loadingOlder') : t('outbound.deliveries.loadOlder') }}
      </Button>
    </CardContent>
    <DeliveryDetailsSheet :delivery-id="openId" @close="openId = null" @changed="load" />
  </Card>
</template>
