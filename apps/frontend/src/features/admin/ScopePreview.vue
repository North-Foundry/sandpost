<script setup lang="ts">
import { onBeforeUnmount, ref, watch } from 'vue'
import { useI18n } from 'vue-i18n'
import { EyeIcon, MailIcon, RefreshCwIcon } from '@lucide/vue'
import type { MessageSummary } from '@/api/contracts'
import { SCOPE_PREVIEW_LIMIT } from '@/api/admin'
import { useSandpostClient } from '@/runtime/client'
import { describeError } from '@/utils/errors'
import { formatListDate, sendersLabel } from '@/utils/format'
import { Button } from '@/components/ui/button'
import { Empty, EmptyDescription, EmptyHeader, EmptyMedia, EmptyTitle } from '@/components/ui/empty'
import { FieldError } from '@/components/ui/field'
import { Skeleton } from '@/components/ui/skeleton'
import { Spinner } from '@/components/ui/spinner'

const props = defineProps<{
  parentId: string | null
  filter: string

  /**
   * Only valid filters are previewed; `null` means validation is still pending.
   */
  valid: boolean | null
}>()
const client = useSandpostClient()
const { t } = useI18n()
const matches = ref<MessageSummary[] | null>(null)
const loading = ref(false)
const error = ref('')
let timer: ReturnType<typeof setTimeout> | null = null
let request = 0

/**
 * Sample the newest matching mail after edits settle.
 */
function schedule(delay = 400): void {
  const current = ++request
  if (timer) clearTimeout(timer)
  if (props.valid !== true) {
    loading.value = props.valid === null
    /*
     * Results for an earlier filter would misrepresent the invalid one.
     */
    if (props.valid === false) { matches.value = null; error.value = '' }
    return
  }
  loading.value = true
  timer = setTimeout(async () => {
    try {
      const result = await client.admin.scopes.preview({ parentId: props.parentId, filter: props.filter })
      if (current !== request) return
      matches.value = result
      error.value = ''
    } catch (failure) {
      if (current === request) error.value = describeError(failure, t('admin.preview.loadError'))
    } finally {
      if (current === request) loading.value = false
    }
  }, delay)
}

watch(() => [props.parentId, props.filter, props.valid], () => schedule(), { immediate: true })
onBeforeUnmount(() => { request += 1; if (timer) clearTimeout(timer) })
</script>

<template>
  <section class="flex flex-col gap-3" aria-labelledby="scope-preview-title">
    <div class="flex items-center gap-2">
      <EyeIcon class="size-4 text-muted-foreground" aria-hidden="true" />
      <h3 id="scope-preview-title" class="text-sm font-medium">{{ t('admin.preview.title') }}</h3>
      <span class="text-xs text-muted-foreground" aria-live="polite">
        <template v-if="loading">{{ t('admin.preview.matching') }}</template>
        <template v-else-if="props.valid === false">{{ t('admin.preview.fixFilter') }}</template>
        <template v-else-if="matches">
          {{ matches.length >= SCOPE_PREVIEW_LIMIT ? t('admin.preview.newestMatches', { count: SCOPE_PREVIEW_LIMIT }) : t(matches.length === 1 ? 'admin.preview.matchCount' : 'admin.preview.matchesCount', { count: matches.length }) }}
        </template>
      </span>
      <Spinner v-if="loading" class="size-3" aria-hidden="true" />
      <Button variant="ghost" size="icon-xs" class="ml-auto" :aria-label="t('admin.preview.refresh')" :disabled="props.valid !== true || loading" @click="schedule(0)"><RefreshCwIcon /></Button>
    </div>
    <p class="text-xs text-muted-foreground">{{ t('admin.preview.description') }}</p>
    <FieldError v-if="error">{{ error }}</FieldError>
    <div v-else-if="!matches && loading" class="flex flex-col gap-1.5">
      <Skeleton v-for="index in 3" :key="index" class="h-10 w-full" />
    </div>
    <Empty v-else-if="matches && !matches.length" class="border py-6">
      <EmptyHeader>
        <EmptyMedia variant="icon"><MailIcon /></EmptyMedia>
        <EmptyTitle class="text-sm">{{ t('admin.preview.emptyTitle') }}</EmptyTitle>
        <EmptyDescription class="text-xs">{{ t('admin.preview.emptyDescription') }}</EmptyDescription>
      </EmptyHeader>
    </Empty>
    <ul v-else-if="matches" :class="['flex max-h-80 flex-col divide-y overflow-y-auto rounded-lg border', loading && 'opacity-60']">
      <li v-for="message in matches" :key="message.id" class="flex min-w-0 items-baseline gap-3 px-3 py-2">
        <span class="flex min-w-0 flex-1 flex-col">
          <span class="truncate text-sm">{{ message.subject || t('admin.preview.noSubject') }}</span>
          <span class="truncate text-xs text-muted-foreground">{{ sendersLabel(message.from) }} → {{ (message.envelopeTo?.length ? message.envelopeTo : message.to).join(', ') }}</span>
        </span>
        <time class="shrink-0 text-xs text-muted-foreground tabular-nums" :datetime="new Date(message.receivedAt).toISOString()">{{ formatListDate(message.receivedAt) }}</time>
      </li>
    </ul>
  </section>
</template>
