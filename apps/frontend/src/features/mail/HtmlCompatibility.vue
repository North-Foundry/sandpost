<script lang="ts">
import type { CompatibilitySummary, HtmlCompatibilityReport } from '@/api/contracts'

/**
 * Reports fetched in this session, shared by every instance because the tab unmounts when hidden;
 * a message's content never changes, so neither does its report.
 */
const reportCache = new Map<string, HtmlCompatibilityReport>()
const REPORT_CACHE_LIMIT = 20
const FILTER_STORAGE_KEY = 'sandpost.compatibilityFilter'
</script>

<script setup lang="ts">
import { useI18n } from 'vue-i18n'
import { computed, ref, watch } from 'vue'
import { ChevronRightIcon, InfoIcon, RotateCwIcon, TriangleAlertIcon } from '@lucide/vue'
import { useSandpostClient } from '@/runtime/client'
import { describeError } from '@/utils/errors'
import { cn } from '@/lib/utils'
import { Alert, AlertAction, AlertDescription } from '@/components/ui/alert'
import { Button } from '@/components/ui/button'
import { Checkbox } from '@/components/ui/checkbox'
import { Collapsible, CollapsibleContent, CollapsibleTrigger } from '@/components/ui/collapsible'
import { Empty, EmptyDescription, EmptyHeader, EmptyMedia, EmptyTitle } from '@/components/ui/empty'
import { Spinner } from '@/components/ui/spinner'
import { ToggleGroup, ToggleGroupItem } from '@/components/ui/toggle-group'
import HtmlCompatibilityFeature from './HtmlCompatibilityFeature.vue'
import {
  PLATFORM_GROUPS,
  featureCounts,
  featureImpact,
  featureResults,
  featureSeverity,
  featureWeight,
  parseCompatibilityFilter,
  selectedClients,
  sortFeatures,
  summarizeReport,
  weightedScore,
  type ClientScope,
  type CompatibilityFilter,
  type FeatureSeverity,
  type PlatformGroup,
} from './htmlCompatibility'

const props = defineProps<{ messageId: string }>()
const { t, locale } = useI18n()
const client = useSandpostClient()

const report = ref<HtmlCompatibilityReport | null>(null)
const loading = ref(false)
const error = ref('')
const filter = ref<CompatibilityFilter>(readFilter())
let request = 0

/**
 * Restore the platform filter, tolerating unavailable storage.
 */
function readFilter(): CompatibilityFilter {
  try {
    return parseCompatibilityFilter(window.localStorage.getItem(FILTER_STORAGE_KEY))
  } catch {
    return parseCompatibilityFilter(null)
  }
}

/**
 * Load the report of the current message, ignoring responses for messages no longer shown.
 */
async function load(): Promise<void> {
  const messageId = props.messageId
  const cached = reportCache.get(messageId)
  if (cached) {
    report.value = cached
    error.value = ''
    return
  }
  const current = ++request
  loading.value = true
  error.value = ''
  report.value = null
  try {
    const loaded = await client.messages.htmlCheck(messageId)
    if (current !== request) return
    if (reportCache.size >= REPORT_CACHE_LIMIT) reportCache.delete(reportCache.keys().next().value!)
    reportCache.set(messageId, loaded)
    report.value = loaded
  } catch (failure) {
    if (current === request) error.value = describeError(failure, t('mail.compatibility.failed'))
  } finally {
    if (current === request) loading.value = false
  }
}

watch(() => props.messageId, load, { immediate: true })
watch(filter, (value) => {
  try {
    window.localStorage.setItem(FILTER_STORAGE_KEY, JSON.stringify(value))
  } catch {
    // The filter is a convenience; it simply resets when storage is unavailable.
  }
}, { deep: true })

const clients = computed(() => report.value ? selectedClients(report.value, filter.value) : [])
const summary = computed(() => report.value ? summarizeReport(report.value, filter.value) : null)
const weighted = computed(() => report.value ? weightedScore(report.value, filter.value) : null)

/**
 * Detected features with their per-client results and weighted impact under the filter, scored and
 * most impactful first.
 */
const features = computed(() => sortFeatures((report.value?.features ?? []).map((feature) => {
  const results = featureResults(feature, clients.value, filter.value)
  const counts = featureCounts(results)
  return { feature, results, counts, impact: featureImpact(counts, featureWeight(feature, report.value!)) }
})))

/**
 * Return the features placed at one severity under the current filter.
 */
function featuresAt(severity: FeatureSeverity) {
  return features.value.filter(({ feature, counts }) => featureSeverity(feature, counts) === severity)
}

const problemFeatures = computed(() => featuresAt('problem'))
const differenceFeatures = computed(() => featuresAt('difference'))
const unscoredFeatures = computed(() => featuresAt('unscored'))
const supportedFeatures = computed(() => featuresAt('supported'))

/**
 * Format a percentage without decimals for display.
 */
function percent(value: number): string {
  return String(Math.round(value))
}

const updated = computed(() => {
  const date = new Date(report.value?.data.updated.split(' ')[0] ?? '')
  return Number.isNaN(date.getTime()) ? report.value?.data.updated ?? '' : new Intl.DateTimeFormat(locale.value, { dateStyle: 'medium' }).format(date)
})

/**
 * Accept the client scope toggle group's value, ignoring attempts to deselect it.
 */
function setScope(value: unknown): void {
  if (value === 'main' || value === 'all') filter.value = { ...filter.value, scope: value as ClientScope }
}

/**
 * Accept the platform toggle group's array value, keeping at least one platform selected so the
 * report never empties.
 */
function setPlatforms(value: unknown): void {
  if (!Array.isArray(value)) return
  const platforms = PLATFORM_GROUPS.filter((group) => value.includes(group))
  if (platforms.length) filter.value = { ...filter.value, platforms }
}

/**
 * Return the stacked bar segments of a set of counts, as percentages of their total.
 */
function barSegments(summary: CompatibilitySummary) {
  const total = summary.identical + summary.withDifferences + summary.withProblems + summary.untested || 1
  return [
    { key: 'identical', width: (summary.identical / total) * 100, class: 'bg-success' },
    { key: 'differences', width: (summary.withDifferences / total) * 100, class: 'bg-success/45' },
    { key: 'problems', width: (summary.withProblems / total) * 100, class: 'bg-destructive' },
    { key: 'untested', width: (summary.untested / total) * 100, class: 'bg-muted-foreground/25' },
  ].filter((segment) => segment.width > 0)
}

/**
 * Return the platform group label.
 */
function platformLabel(group: PlatformGroup): string {
  return t(`mail.compatibility.platform.${group}`)
}
</script>

<template>
  <div class="flex flex-col gap-4" :aria-busy="loading">
    <Empty v-if="loading" role="status">
      <EmptyHeader>
        <EmptyMedia variant="icon"><Spinner /></EmptyMedia>
        <EmptyTitle>{{ t('mail.compatibility.checking') }}</EmptyTitle>
      </EmptyHeader>
    </Empty>

    <Alert v-else-if="error" variant="destructive" class="pr-24" role="alert">
      <AlertDescription>{{ error }}</AlertDescription>
      <AlertAction><Button variant="outline" size="xs" @click="load"><RotateCwIcon data-icon="inline-start" />{{ t('mail.detail.retry') }}</Button></AlertAction>
    </Alert>

    <Empty v-else-if="report && !report.htmlPresent">
      <EmptyHeader>
        <EmptyTitle>{{ t('mail.compatibility.noHtmlTitle') }}</EmptyTitle>
        <EmptyDescription>{{ t('mail.compatibility.noHtmlDescription') }}</EmptyDescription>
      </EmptyHeader>
    </Empty>

    <template v-else-if="report && summary">
      <section class="flex flex-col gap-3 rounded-lg border p-4" :aria-label="t('mail.compatibility.summaryLabel')">
        <p class="text-base font-semibold tabular-nums">
          {{ t('mail.compatibility.worksIn', { works: summary.identical + summary.withDifferences, clients: summary.clients }) }}
        </p>
        <div v-if="summary.clients" class="flex h-2 overflow-hidden rounded-full bg-muted" aria-hidden="true">
          <span v-for="segment in barSegments(summary)" :key="segment.key" :class="segment.class" :style="{ width: `${segment.width}%` }" />
        </div>
        <ul v-if="summary.clients" class="flex flex-wrap gap-x-4 gap-y-1 text-xs text-muted-foreground tabular-nums">
          <li class="flex items-center gap-1.5"><span class="size-2 rounded-full bg-success" aria-hidden="true" />{{ t('mail.compatibility.identicalCount', { count: summary.identical }) }}</li>
          <li class="flex items-center gap-1.5"><span class="size-2 rounded-full bg-success/45" aria-hidden="true" />{{ t('mail.compatibility.differencesCount', { count: summary.withDifferences }) }}</li>
          <li class="flex items-center gap-1.5"><span class="size-2 rounded-full bg-destructive" aria-hidden="true" />{{ t('mail.compatibility.problemsCount', { count: summary.withProblems }) }}</li>
          <li v-if="summary.untested" class="flex items-center gap-1.5"><span class="size-2 rounded-full bg-muted-foreground/25" aria-hidden="true" />{{ t('mail.compatibility.untestedCount', { count: summary.untested }) }}</li>
        </ul>
        <p v-if="summary.clients && weighted" class="flex items-center gap-1.5 text-xs text-muted-foreground tabular-nums">
          <span class="font-medium text-foreground/80">{{ t('mail.compatibility.weightedLabel') }}</span>
          {{ t('mail.compatibility.weightedValues', { supported: percent(weighted.supported), partial: percent(weighted.partial), unsupported: percent(weighted.unsupported) }) }}
          <InfoIcon class="size-3.5 shrink-0 cursor-help" :aria-label="t('mail.compatibility.weightedHelp')" role="img" :title="t('mail.compatibility.weightedHelp')" />
        </p>
        <div class="flex flex-wrap items-center gap-3 border-t pt-3">
          <ToggleGroup type="single" variant="outline" size="sm" :model-value="filter.scope" :aria-label="t('mail.compatibility.scopeLabel')" @update:model-value="setScope">
            <ToggleGroupItem v-for="scope in (['main', 'all'] as const)" :key="scope" :value="scope" class="h-7 px-2.5 text-xs data-[state=off]:bg-transparent data-[state=off]:text-muted-foreground">{{ t(`mail.compatibility.scope.${scope}`) }}</ToggleGroupItem>
          </ToggleGroup>
          <ToggleGroup type="multiple" variant="outline" size="sm" :model-value="filter.platforms" :aria-label="t('mail.compatibility.platforms')" @update:model-value="setPlatforms">
            <ToggleGroupItem v-for="group in PLATFORM_GROUPS" :key="group" :value="group" class="h-7 px-2.5 text-xs data-[state=off]:bg-transparent data-[state=off]:text-muted-foreground data-[state=off]:line-through data-[state=off]:decoration-muted-foreground/50">{{ platformLabel(group) }}</ToggleGroupItem>
          </ToggleGroup>
          <label class="ml-auto flex items-center gap-2 text-xs text-muted-foreground">
            <Checkbox :model-value="filter.olderVersions" @update:model-value="filter = { ...filter, olderVersions: $event === true }" />
            {{ t('mail.compatibility.olderVersions') }}
          </label>
        </div>
      </section>

      <Alert v-if="report.externalStylesheets">
        <TriangleAlertIcon />
        <AlertDescription>{{ t('mail.compatibility.externalStylesheets', { count: report.externalStylesheets }) }}</AlertDescription>
      </Alert>

      <section v-if="summary.clients" class="flex flex-col gap-2" :aria-label="t('mail.compatibility.problemFeatures')">
        <h3 class="text-xs font-medium text-muted-foreground">{{ t('mail.compatibility.problemFeatures') }}</h3>
        <p v-if="!problemFeatures.length" class="rounded-lg border border-dashed px-3 py-4 text-center text-muted-foreground">{{ t('mail.compatibility.noProblems') }}</p>
        <HtmlCompatibilityFeature v-for="entry in problemFeatures" :key="entry.feature.slug" v-bind="entry" />
      </section>

      <section v-if="summary.clients && differenceFeatures.length" class="flex flex-col gap-2" :aria-label="t('mail.compatibility.differenceFeatures')">
        <h3 class="text-xs font-medium text-muted-foreground">{{ t('mail.compatibility.differenceFeatures') }}</h3>
        <p class="text-xs text-muted-foreground">{{ t('mail.compatibility.differenceHelp') }}</p>
        <HtmlCompatibilityFeature v-for="entry in differenceFeatures" :key="entry.feature.slug" v-bind="entry" />
      </section>


      <Collapsible v-if="summary.clients && unscoredFeatures.length" v-slot="{ open: sectionOpen }" class="flex flex-col gap-2">
        <CollapsibleTrigger class="flex items-center gap-1.5 rounded-md text-xs font-medium text-muted-foreground outline-none hover:text-foreground focus-visible:ring-2 focus-visible:ring-ring">
          <ChevronRightIcon :class="cn('size-3.5 transition-transform', sectionOpen && 'rotate-90')" aria-hidden="true" />
          {{ t('mail.compatibility.unscoredFeatures', { count: unscoredFeatures.length }) }}
        </CollapsibleTrigger>
        <CollapsibleContent class="flex flex-col gap-2">
          <p class="text-xs text-muted-foreground">{{ t('mail.compatibility.unscoredHelp') }}</p>
          <HtmlCompatibilityFeature v-for="entry in unscoredFeatures" :key="entry.feature.slug" v-bind="entry" />
        </CollapsibleContent>
      </Collapsible>

      <Collapsible v-if="summary.clients && supportedFeatures.length" v-slot="{ open }">
        <CollapsibleTrigger class="flex items-center gap-1.5 rounded-md text-xs font-medium text-muted-foreground outline-none hover:text-foreground focus-visible:ring-2 focus-visible:ring-ring">
          <ChevronRightIcon :class="cn('size-3.5 transition-transform', open && 'rotate-90')" aria-hidden="true" />
          {{ t('mail.compatibility.supportedFeatures', { count: supportedFeatures.length }) }}
        </CollapsibleTrigger>
        <CollapsibleContent>
          <ul class="mt-2 flex flex-wrap gap-1.5">
            <li v-for="{ feature } in supportedFeatures" :key="feature.slug">
              <a :href="feature.url" target="_blank" rel="noopener noreferrer" class="flex items-center gap-1.5 rounded-md border px-2 py-0.5 text-xs hover:bg-muted/40">
                <span class="size-1.5 rounded-full bg-success" aria-hidden="true" />{{ feature.title }}
              </a>
            </li>
          </ul>
        </CollapsibleContent>
      </Collapsible>

      <p class="text-xs text-muted-foreground/70">
        <a :href="report.data.url" target="_blank" rel="noopener noreferrer" class="underline underline-offset-2">{{ report.data.source }}</a>
        {{ t('mail.compatibility.dataSource', { license: report.data.license, date: updated }) }}
      </p>
    </template>
  </div>
</template>
