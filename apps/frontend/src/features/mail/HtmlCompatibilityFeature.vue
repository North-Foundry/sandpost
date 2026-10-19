<script setup lang="ts">
import { useI18n } from 'vue-i18n'
import { ChevronRightIcon, ExternalLinkIcon } from '@lucide/vue'
import type { CompatibilityFeature } from '@/api/contracts'
import { cn } from '@/lib/utils'
import { Badge } from '@/components/ui/badge'
import { Collapsible, CollapsibleContent, CollapsibleTrigger } from '@/components/ui/collapsible'
import { markdownSegments, type FeatureClientResult, type FeatureCounts } from './htmlCompatibility'

const props = defineProps<{
  feature: CompatibilityFeature
  results: FeatureClientResult[]
  counts: FeatureCounts

  /**
   * Weighted impact in percentage points; shown only for scored features.
   */
  impact: number
}>()
const { t } = useI18n()

/**
 * Return the stacked bar segments of the feature's support counts, as percentages of their total.
 */
function barSegments(counts: FeatureCounts) {
  const total = counts.yes + counts.partial + counts.no + counts.unknown || 1
  return [
    { key: 'supported', width: (counts.yes / total) * 100, class: 'bg-success' },
    { key: 'partial', width: (counts.partial / total) * 100, class: 'bg-warning' },
    { key: 'unsupported', width: (counts.no / total) * 100, class: unsupportedColor() },
    { key: 'untested', width: (counts.unknown / total) * 100, class: 'bg-muted-foreground/25' },
  ].filter((segment) => segment.width > 0)
}

/**
 * Color unsupported results red only when they break the message; graceful features merely lose
 * an enhancement there.
 */
function unsupportedColor(): string {
  return props.feature.graceful ? 'bg-warning' : 'bg-destructive'
}

/**
 * Describe a client and the versions behind its result.
 */
function clientLabel(result: FeatureClientResult): string {
  const versions = result.versions.length > 2 ? `${result.versions[0]}–${result.versions.at(-1)}` : result.versions.join(', ')
  return `${result.client.familyName} ${result.client.platformName}${versions ? ` (${versions})` : ''}`
}

/**
 * Return the label of a caniemail category, falling back to its key.
 */
function categoryLabel(category: string): string {
  return ['html', 'css', 'image'].includes(category) ? t(`mail.compatibility.category.${category}`) : category
}

/**
 * Format a percentage without decimals for display.
 */
function percent(value: number): string {
  return String(Math.round(value))
}
</script>

<template>
  <Collapsible v-slot="{ open }" class="rounded-lg border">
    <CollapsibleTrigger class="flex w-full items-center gap-2 rounded-lg px-3 py-2.5 text-left outline-none hover:bg-muted/40 focus-visible:ring-2 focus-visible:ring-ring">
      <ChevronRightIcon :class="cn('size-4 shrink-0 text-muted-foreground transition-transform', open && 'rotate-90')" aria-hidden="true" />
      <span class="min-w-0 flex-1 truncate font-medium">{{ feature.title }}</span>
      <Badge variant="secondary">{{ categoryLabel(feature.category) }}</Badge>
      <span class="w-10 shrink-0 text-right text-xs text-muted-foreground tabular-nums" :title="t('mail.compatibility.occurrencesTitle', { count: feature.occurrences })">{{ t('mail.compatibility.occurrences', { count: feature.occurrences }) }}</span>
      <span v-if="feature.scored" class="w-9 shrink-0 text-right text-xs text-muted-foreground tabular-nums" :title="t('mail.compatibility.impactTitle')">{{ t('mail.compatibility.impact', { impact: percent(impact) }) }}</span>
      <span class="flex h-1.5 w-20 shrink-0 overflow-hidden rounded-full bg-muted max-sm:hidden" aria-hidden="true">
        <span v-for="segment in barSegments(props.counts)" :key="segment.key" :class="segment.class" :style="{ width: `${segment.width}%` }" />
      </span>
      <span class="sr-only">{{ t('mail.compatibility.featureCounts', { unsupported: counts.no, partial: counts.partial }) }}</span>
    </CollapsibleTrigger>
    <CollapsibleContent class="flex flex-col gap-3 border-t px-3 py-3 text-sm">
      <p v-if="feature.description" class="text-muted-foreground">
        <template v-for="(segment, index) in markdownSegments(feature.description)" :key="index">
          <code v-if="segment.kind === 'code'" class="rounded bg-muted px-1 font-mono text-xs">{{ segment.text }}</code>
          <a v-else-if="segment.kind === 'link'" :href="segment.href" target="_blank" rel="noopener noreferrer" class="underline underline-offset-2">{{ segment.text }}</a>
          <template v-else>{{ segment.text }}</template>
        </template>
      </p>
      <div v-if="feature.examples.length" class="flex flex-col gap-1">
        <span class="text-xs font-medium text-muted-foreground">{{ t('mail.compatibility.examples') }}</span>
        <div class="flex flex-wrap gap-1.5">
          <code v-for="example in feature.examples" :key="example" class="max-w-full truncate rounded bg-muted px-1.5 py-0.5 font-mono text-xs">{{ example }}</code>
        </div>
      </div>
      <p v-if="feature.graceful && feature.scored" class="text-xs text-muted-foreground">{{ t('mail.compatibility.gracefulNote') }}</p>
      <p v-if="feature.selectorUnverified" class="text-xs text-muted-foreground">{{ t('mail.compatibility.selectorUnverified') }}</p>
      <div v-for="level in (['no', 'partial'] as const)" :key="level" class="flex flex-col gap-1">
        <template v-if="results.some((result) => result.support === level)">
          <span class="text-xs font-medium text-muted-foreground">{{ level === 'partial' ? t('mail.compatibility.partialClients') : feature.graceful ? t('mail.compatibility.ignoredClients') : t('mail.compatibility.unsupportedClients') }}</span>
          <ul class="flex flex-wrap gap-1.5">
            <li v-for="result in results.filter((candidate) => candidate.support === level)" :key="result.client.id" class="flex items-center gap-1.5 rounded-md border px-2 py-0.5 text-xs">
              <span :class="cn('size-1.5 shrink-0 rounded-full', level === 'no' ? unsupportedColor() : 'bg-warning')" aria-hidden="true" />
              {{ clientLabel(result) }}
              <sup v-if="result.notes.length" class="text-muted-foreground">{{ result.notes.join(',') }}</sup>
            </li>
          </ul>
        </template>
      </div>
      <ol v-if="Object.keys(feature.notes).length" class="flex flex-col gap-1 text-xs text-muted-foreground">
        <li v-for="(note, number) in feature.notes" :key="number" class="flex gap-1.5">
          <span class="tabular-nums">{{ number }}.</span>
          <span>
            <template v-for="(segment, index) in markdownSegments(note)" :key="index">
              <code v-if="segment.kind === 'code'" class="rounded bg-muted px-1 font-mono">{{ segment.text }}</code>
              <a v-else-if="segment.kind === 'link'" :href="segment.href" target="_blank" rel="noopener noreferrer" class="underline underline-offset-2">{{ segment.text }}</a>
              <template v-else>{{ segment.text }}</template>
            </template>
          </span>
        </li>
      </ol>
      <a :href="feature.url" target="_blank" rel="noopener noreferrer" class="flex w-fit items-center gap-1 text-xs underline underline-offset-2">
        {{ t('mail.compatibility.reference') }}<ExternalLinkIcon class="size-3" aria-hidden="true" />
      </a>
    </CollapsibleContent>
  </Collapsible>
</template>
