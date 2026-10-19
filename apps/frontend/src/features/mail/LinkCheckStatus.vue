<script setup lang="ts">
import { useI18n } from 'vue-i18n'
import { computed, ref } from 'vue'
import { ChevronRightIcon, LinkIcon, RotateCwIcon } from '@lucide/vue'
import type { LinkCheckReport, LinkOutcome } from '@/api/contracts'
import { cn } from '@/lib/utils'
import { formatFullDate } from '@/utils/format'
import { Button } from '@/components/ui/button'
import { Popover, PopoverContent, PopoverTrigger } from '@/components/ui/popover'
import { Spinner } from '@/components/ui/spinner'
import {
  OUTCOMES_BY_CONCERN,
  TONE_DOTS,
  displayUrl,
  formatRelativeTime,
  outcomeTone,
  shortStatus,
  sortByConcern,
  summaryParts,
  type LinkTone,
} from './linkCheck'

const props = defineProps<{
  report: LinkCheckReport | null
  running: boolean
  error: string
  now: number
}>()
const emit = defineEmits<{
  /**
   * Requests a new check of the message's links.
   */
  (event: 'check'): void

  /**
   * Requests the Links tab with every link's full result.
   */
  (event: 'details'): void
}>()
const { t, locale } = useI18n()
const open = ref(false)

const hasLinks = computed(() => Boolean(props.report?.urls.length))
const checked = computed(() => props.report?.checkedAt !== null && props.report?.checkedAt !== undefined)

/**
 * The most concerning outcome present, which colors the summary's single status dot.
 */
const worstTone = computed<LinkTone | null>(() => {
  const present = new Set(props.report?.links.map((link) => link.outcome))
  const worst = OUTCOMES_BY_CONCERN.find((outcome) => present.has(outcome))
  return worst ? outcomeTone(worst) : null
})

/**
 * Count labels such as "4 healthy" or "2 server errors", in reading order.
 */
const countLabels = computed(() => props.report
  ? summaryParts(props.report).map(({ outcome, count }) => ({ outcome, label: countLabel(outcome, count) }))
  : [])

const totalLabel = computed(() => props.report
  ? t(props.report.summary.total === 1 ? 'mail.links.total_one' : 'mail.links.total_other', { count: props.report.summary.total })
  : '')

const checkedRelative = computed(() => props.report?.checkedAt
  ? formatRelativeTime(props.report.checkedAt, props.now, locale.value)
  : '')
const checkedAbsolute = computed(() => props.report?.checkedAt ? formatFullDate(props.report.checkedAt * 1000) : '')

/**
 * The compact summary, for example "7 links checked · 4 healthy · 2 broken · 1 failed".
 */
const summary = computed(() => [totalLabel.value, ...countLabels.value.map(({ label }) => label)].join(' · '))

/**
 * Checked links, most concerning first, each with only its color and short status.
 */
const rows = computed(() => sortByConcern(props.report?.links ?? []).map((link) => ({
  link,
  tone: outcomeTone(link.outcome),
  status: shortStatus(link, t),
})))

/**
 * Return the label of one outcome count, choosing the plural form where the language needs one.
 */
function countLabel(outcome: LinkOutcome, count: number): string {
  if (outcome === 'server_error') {
    return t(count === 1 ? 'mail.links.count.server_error_one' : 'mail.links.count.server_error_other', { count })
  }
  return t(`mail.links.count.${outcome}`, { count })
}
</script>

<template>
  <div v-if="hasLinks" class="flex min-w-0 shrink-0 items-center justify-end">
    <span v-if="!checked && props.running" class="flex items-center gap-1.5 px-1" role="status">
      <Spinner class="size-3" aria-hidden="true" />{{ t('mail.links.checking') }}
    </span>
    <Button
      v-else-if="!checked"
      variant="ghost"
      size="xs"
      class="-my-1 text-muted-foreground"
      :title="t('mail.links.checkTitle')"
      @click="emit('check')"
    >
      <LinkIcon data-icon="inline-start" />{{ t('mail.links.check') }}
    </Button>
    <span v-else-if="!checked && props.error" class="truncate text-destructive" role="alert">{{ props.error }}</span>

    <Popover v-else-if="checked && props.report" v-model:open="open">
      <PopoverTrigger as-child>
        <Button variant="ghost" size="xs" class="-my-1 min-w-0 max-w-full text-muted-foreground" :aria-label="`${t('mail.links.summaryLabel')}: ${summary}`">
          <Spinner v-if="props.running" class="size-3" aria-hidden="true" />
          <span v-else-if="worstTone" :class="cn('size-1.5 shrink-0 rounded-full', TONE_DOTS[worstTone])" aria-hidden="true" />
          <span class="truncate" aria-hidden="true">{{ totalLabel }}<!--
            Narrow screens keep the problem counts and drop the healthy count.
            --><span v-for="part in countLabels" :key="part.outcome" :class="part.outcome === 'healthy' && 'max-sm:hidden'"> · {{ part.label }}</span></span>
          <span class="shrink-0 max-sm:hidden" :title="checkedAbsolute">· {{ checkedRelative }}</span>
        </Button>
      </PopoverTrigger>
      <PopoverContent align="end" class="w-80 gap-0 overflow-hidden p-0">
        <div class="flex items-center justify-between gap-3 border-b px-3 py-2">
          <div class="flex min-w-0 flex-col">
            <p class="font-medium">{{ t('mail.links.title') }}</p>
            <p class="text-xs text-muted-foreground">
              <time :datetime="new Date(props.report.checkedAt! * 1000).toISOString()" :title="checkedAbsolute">{{ t('mail.links.checkedAt', { time: checkedRelative }) }}</time>
            </p>
          </div>
          <Button variant="outline" size="xs" :disabled="props.running" @click="emit('check')">
            <Spinner v-if="props.running" data-icon="inline-start" aria-hidden="true" /><RotateCwIcon v-else data-icon="inline-start" />
            {{ props.running ? t('mail.links.checking') : t('mail.links.recheck') }}
          </Button>
        </div>
        <p v-if="props.error" class="border-b px-3 py-2 text-xs text-destructive" role="alert">{{ props.error }}</p>
        <ul class="max-h-64 overflow-y-auto py-1" :aria-label="t('mail.links.summaryLabel')">
          <li v-for="row in rows" :key="row.link.url" class="flex items-center gap-2.5 px-3 py-1">
            <span :class="cn('size-1.5 shrink-0 rounded-full', TONE_DOTS[row.tone])" aria-hidden="true" />
            <span class="min-w-0 flex-1 truncate text-xs" :title="row.link.url">{{ displayUrl(row.link.url) }}</span>
            <span class="shrink-0 text-xs text-muted-foreground tabular-nums">
              <span class="sr-only">{{ t(`mail.links.outcome.${row.link.outcome}`) }}: </span>{{ row.status }}
            </span>
          </li>
        </ul>
        <button
          type="button"
          class="flex w-full items-center justify-between border-t px-3 py-2 text-xs text-muted-foreground outline-none hover:bg-muted hover:text-foreground focus-visible:bg-muted"
          @click="open = false; emit('details')"
        >
          {{ t('mail.links.viewDetails') }}<ChevronRightIcon class="size-3.5" aria-hidden="true" />
        </button>
      </PopoverContent>
    </Popover>
  </div>
</template>
