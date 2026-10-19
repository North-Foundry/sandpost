<script setup lang="ts">
import { useI18n } from 'vue-i18n'
import { computed } from 'vue'
import { LinkIcon, RotateCwIcon } from '@lucide/vue'
import type { LinkCheckReport } from '@/api/contracts'
import { cn } from '@/lib/utils'
import { formatFullDate } from '@/utils/format'
import { Alert, AlertDescription } from '@/components/ui/alert'
import { Button } from '@/components/ui/button'
import { Spinner } from '@/components/ui/spinner'
import {
  TONE_DOTS,
  destination,
  explanation,
  formatRelativeTime,
  outcomeTone,
  resultsByUrl,
  shortStatus,
} from './linkCheck'

const props = defineProps<{
  report: LinkCheckReport
  running: boolean
  error: string
  now: number
}>()
const emit = defineEmits<{
  /**
   * Requests a new check of the message's links.
   */
  (event: 'check'): void
}>()
const { t, locale } = useI18n()

const checked = computed(() => props.report.checkedAt !== null)
const checkedRelative = computed(() => props.report.checkedAt === null ? '' : formatRelativeTime(props.report.checkedAt, props.now, locale.value))
const checkedAbsolute = computed(() => props.report.checkedAt === null ? '' : formatFullDate(props.report.checkedAt * 1000))

/**
 * Every link of the message in message order, with its full stored result when it has one.
 */
const rows = computed(() => {
  const results = resultsByUrl(props.report)
  return props.report.urls.map((url) => {
    const result = results.get(url)
    if (!result) return { url, result: null }
    return {
      url,
      result,
      tone: outcomeTone(result.outcome),
      status: shortStatus(result, t),
      verdict: result.errorKind
        ? `${t(`mail.links.outcome.${result.outcome}`)} · ${t(`mail.links.error.${result.errorKind}`)}`
        : t(`mail.links.outcome.${result.outcome}`),
      response: result.statusCode === null
        ? t('mail.links.noResponse')
        : result.statusText
          ? t('mail.links.status.http', { code: result.statusCode, text: result.statusText })
          : t('mail.links.status.httpCode', { code: result.statusCode }),
      explanation: explanation(result, t),
      destination: destination(result, t),
      checked: formatRelativeTime(result.checkedAt, props.now, locale.value),
      checkedAbsolute: formatFullDate(result.checkedAt * 1000),
    }
  })
})
</script>

<template>
  <div class="flex flex-col gap-3">
    <div class="flex items-center justify-between gap-3">
      <span class="text-xs text-muted-foreground">
        {{ t(props.report.urls.length === 1 ? 'mail.links.found_one' : 'mail.links.found_other', { count: props.report.urls.length }) }} ·
        <time v-if="checked" :datetime="new Date(props.report.checkedAt! * 1000).toISOString()" :title="checkedAbsolute">{{ t('mail.links.checkedAt', { time: checkedRelative }) }}</time>
        <template v-else>{{ t('mail.links.notChecked') }}</template>
      </span>
      <Button variant="ghost" size="sm" :disabled="props.running" @click="emit('check')">
        <Spinner v-if="props.running" data-icon="inline-start" aria-hidden="true" />
        <RotateCwIcon v-else-if="checked" data-icon="inline-start" />
        <LinkIcon v-else data-icon="inline-start" />
        {{ props.running ? t('mail.links.checking') : checked ? t('mail.links.recheck') : t('mail.links.check') }}
      </Button>
    </div>
    <Alert v-if="props.error" variant="destructive" role="alert"><AlertDescription>{{ props.error }}</AlertDescription></Alert>

    <ul class="divide-y overflow-hidden rounded-lg border" :aria-label="t('mail.links.summaryLabel')" :aria-busy="props.running">
      <li v-for="row in rows" :key="row.url" class="flex flex-col gap-2 px-3 py-2.5">
        <div class="flex items-start gap-2.5">
          <span :class="cn('mt-1.5 size-1.5 shrink-0 rounded-full', row.result ? TONE_DOTS[row.tone!] : TONE_DOTS.muted)" aria-hidden="true" />
          <span class="min-w-0 flex-1 font-mono text-xs/relaxed break-all">{{ row.url }}</span>
          <span v-if="row.result" class="shrink-0 text-xs font-medium tabular-nums">{{ row.status }}</span>
        </div>
        <dl v-if="row.result" class="ml-4 grid grid-cols-[minmax(5.5rem,auto)_minmax(0,1fr)] gap-x-3 gap-y-0.5 text-xs">
          <dt class="text-muted-foreground">{{ t('mail.links.field.result') }}</dt>
          <dd>{{ row.verdict }}</dd>
          <dt class="text-muted-foreground">{{ t('mail.links.field.response') }}</dt>
          <dd class="tabular-nums">{{ row.response }}</dd>
          <dt class="text-muted-foreground">{{ t('mail.links.field.meaning') }}</dt>
          <dd>{{ row.explanation }}</dd>
          <template v-if="row.destination">
            <dt class="text-muted-foreground">{{ t('mail.links.field.redirect') }}</dt>
            <dd class="break-all">{{ row.destination }}</dd>
          </template>
          <template v-if="row.result.errorMessage">
            <dt class="text-muted-foreground">{{ t('mail.links.field.error') }}</dt>
            <dd class="font-mono break-all">{{ row.result.errorMessage }}</dd>
          </template>
          <dt class="text-muted-foreground">{{ t('mail.links.field.checked') }}</dt>
          <dd><time :datetime="new Date(row.result.checkedAt * 1000).toISOString()">{{ row.checked }} · {{ row.checkedAbsolute }}</time></dd>
        </dl>
      </li>
    </ul>
    <p v-if="props.report.omittedLinks" class="text-xs text-muted-foreground">
      {{ t(props.report.omittedLinks === 1 ? 'mail.links.omitted_one' : 'mail.links.omitted_other', { count: props.report.omittedLinks }) }}
    </p>
    <p class="text-xs text-muted-foreground">{{ t('mail.links.description') }}</p>
  </div>
</template>
