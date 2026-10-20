<script setup lang="ts">
import { computed, onBeforeUnmount, ref, watch } from 'vue'
import { useI18n } from 'vue-i18n'
import {
  ArrowLeftIcon, ChevronRightIcon, GitBranchIcon, MailOpenIcon, MousePointerClickIcon, RefreshCwIcon, TimerIcon,
} from '@lucide/vue'
import type { Persona, PersonaExecutionAction, PersonaExecutionDetails, PersonaExecutionSummary, PersonaHttpRequest } from '@/api/contracts'
import { useSandpostClient } from '@/runtime/client'
import { describeError } from '@/utils/errors'
import { formatBytes, formatFullDate, formatStamp } from '@/utils/format'
import { cn } from '@/lib/utils'
import { Alert, AlertDescription } from '@/components/ui/alert'
import { Badge } from '@/components/ui/badge'
import { Button } from '@/components/ui/button'
import { Empty, EmptyDescription, EmptyHeader, EmptyTitle } from '@/components/ui/empty'
import { Sheet, SheetContent, SheetDescription, SheetHeader, SheetTitle } from '@/components/ui/sheet'
import { Skeleton } from '@/components/ui/skeleton'
import { Spinner } from '@/components/ui/spinner'
import { displayUrl } from '@/features/mail/linkCheck'
import { useWorkspace } from '@/features/shell/workspace'
import { TONE_DOT, actionTone, executionTone, formatDuration, isUnfinished, requestTone, responded } from './format'

const open = defineModel<boolean>('open', { required: true })
const props = defineProps<{ persona: Persona | null }>()
const emit = defineEmits<{ (event: 'open-message', id: string): void }>()
const { t, locale } = useI18n()
const client = useSandpostClient()
const workspace = useWorkspace()
const PAGE_SIZE = 30
const REFRESH_INTERVAL = 2000

const executions = ref<PersonaExecutionSummary[]>([])
const hasOlder = ref(false)
const loading = ref(false)
const loadingOlder = ref(false)
const error = ref('')
const details = ref<PersonaExecutionDetails | null>(null)
const detailsLoading = ref(false)
let refreshTimer: ReturnType<typeof setTimeout> | null = null
let generation = 0
let unsubscribe: (() => void) | null = null

const unfinished = computed(() => details.value
  ? isUnfinished(details.value.status)
  : executions.value.some((execution) => isUnfinished(execution.status)))

/**
 * Load the newest page, or the open execution, ignoring responses from superseded requests.
 */
async function refresh(): Promise<void> {
  const persona = props.persona
  if (!persona || !client.personas) return
  const request = ++generation
  if (!executions.value.length && !details.value) loading.value = true
  try {
    if (details.value) {
      const updated = await client.personas.execution(persona.id, details.value.id)
      if (request === generation) details.value = updated
    } else {
      const page = await client.personas.executions(persona.id, { limit: PAGE_SIZE })
      if (request !== generation) return
      executions.value = page
      hasOlder.value = page.length === PAGE_SIZE
      if (page[0]) workspace.updatePersona({ ...persona, lastExecution: page[0] })
    }
    error.value = ''
  } catch (failure) {
    if (request === generation) error.value = describeError(failure, t('personas.activity.loadFailed'))
  } finally {
    if (request === generation) loading.value = false
    scheduleRefresh()
  }
}

/**
 * Poll while something can still change and the sheet is open.
 */
function scheduleRefresh(): void {
  if (refreshTimer) clearTimeout(refreshTimer)
  refreshTimer = open.value && unfinished.value ? setTimeout(() => { void refresh() }, REFRESH_INTERVAL) : null
}

/**
 * Append the next older page.
 */
async function loadOlder(): Promise<void> {
  const persona = props.persona
  const oldest = executions.value.at(-1)
  if (!persona || !oldest || loadingOlder.value || !client.personas) return
  loadingOlder.value = true
  try {
    const page = await client.personas.executions(persona.id, { before: oldest.sequence, limit: PAGE_SIZE })
    executions.value = [...executions.value, ...page.filter((execution) => !executions.value.some((known) => known.id === execution.id))]
    hasOlder.value = page.length === PAGE_SIZE
  } catch (failure) {
    error.value = describeError(failure, t('personas.activity.loadFailed'))
  } finally { loadingOlder.value = false }
}

/**
 * Open one execution's steps.
 */
async function showDetails(execution: PersonaExecutionSummary): Promise<void> {
  const persona = props.persona
  if (!persona || !client.personas) return
  detailsLoading.value = true
  generation += 1
  try {
    details.value = await client.personas.execution(persona.id, execution.id)
    error.value = ''
  } catch (failure) {
    error.value = describeError(failure, t('personas.activity.loadFailed'))
  } finally {
    detailsLoading.value = false
    scheduleRefresh()
  }
}

/**
 * Return from an execution to the run list.
 */
function showList(): void {
  details.value = null
  void refresh()
}

/**
 * Describe the headline of a step.
 */
function stepTitle(action: PersonaExecutionAction): string {
  const detail = action.detail
  switch (detail.kind) {
    case 'branch': return detail.outcome ? t('personas.step.branchTrue') : t('personas.step.branchFalse')
    case 'open': return t('personas.step.open')
    case 'wait': return t('personas.step.wait', { duration: formatDuration(detail.milliseconds, locale.value) })
    case 'click': return t('personas.step.click', { selector: detail.selector })
  }
}

/**
 * Summarize a request outcome in one short line.
 */
function requestLine(request: PersonaHttpRequest): string {
  const parts = [responded(request) ? t('personas.request.status', { code: request.statusCode! }) : request.error ?? t('personas.request.noResponse')]
  if (request.redirectCount) parts.push(t('personas.request.redirects', { count: request.redirectCount }))
  if (responded(request)) parts.push(formatBytes(request.receivedBytes))
  if (request.truncated) parts.push(t('personas.request.truncated'))
  parts.push(formatDuration(request.durationMilliseconds, locale.value))
  return parts.join(' · ')
}

/**
 * Format a Unix-seconds timestamp compactly.
 */
function stamp(seconds: number): string {
  return formatStamp(seconds * 1000)
}

/**
 * Open the execution's message in the mail view.
 */
function openMessage(id: string): void {
  open.value = false
  emit('open-message', id)
}

watch([open, () => props.persona?.id], ([isOpen]) => {
  generation += 1
  details.value = null
  executions.value = []
  error.value = ''
  unsubscribe?.()
  unsubscribe = null
  if (refreshTimer) clearTimeout(refreshTimer)
  if (!isOpen) return
  void refresh()
  // New mail may schedule runs; refresh the list when it arrives.
  unsubscribe = workspace.onMailEvent(() => { if (!details.value) void refresh() })
}, { immediate: true })

onBeforeUnmount(() => {
  generation += 1
  unsubscribe?.()
  if (refreshTimer) clearTimeout(refreshTimer)
})

const ACTION_ICONS = { branch: GitBranchIcon, open: MailOpenIcon, wait: TimerIcon, click: MousePointerClickIcon } as const
</script>

<template>
  <Sheet v-model:open="open">
    <SheetContent side="right" class="flex w-full flex-col gap-0 p-0 sm:max-w-lg">
      <SheetHeader class="border-b px-4 py-3">
        <SheetTitle>{{ t('personas.activity.title', { name: props.persona?.name ?? '' }) }}</SheetTitle>
        <SheetDescription>{{ t('personas.activity.description') }}</SheetDescription>
      </SheetHeader>

      <div class="min-h-0 flex-1 overflow-y-auto">
        <Alert v-if="error" variant="destructive" class="m-4 w-auto" role="alert"><AlertDescription>{{ error }}</AlertDescription></Alert>

        <template v-if="details">
          <div class="flex flex-col gap-3 border-b px-4 py-3">
            <Button variant="ghost" size="sm" class="-ml-2 w-fit" @click="showList"><ArrowLeftIcon data-icon="inline-start" />{{ t('personas.activity.back') }}</Button>
            <div class="flex min-w-0 items-start gap-2">
              <span :class="cn('mt-1.5 size-2 shrink-0 rounded-full', TONE_DOT[executionTone(details.status)])" aria-hidden="true" />
              <div class="flex min-w-0 flex-1 flex-col gap-0.5">
                <span class="truncate text-sm font-medium">{{ details.subject || t('personas.activity.noSubject') }}</span>
                <span class="text-xs text-muted-foreground">
                  {{ t(`personas.status.${details.status}`) }} · <time :title="formatFullDate(details.scheduledAt * 1000)">{{ t('personas.activity.scheduled', { time: stamp(details.scheduledAt) }) }}</time>
                  <template v-if="details.completedAt"> · {{ t('personas.activity.finished', { time: stamp(details.completedAt) }) }}</template>
                </span>
              </div>
              <Button variant="outline" size="xs" class="shrink-0" @click="openMessage(details.messageId)">{{ t('personas.activity.openMessage') }}</Button>
            </div>
            <p v-if="details.error" class="text-xs break-words text-destructive">{{ details.error }}</p>
          </div>
          <section class="flex flex-col px-4 py-3" :aria-label="t('personas.activity.steps')">
            <p v-if="!details.actions.length" class="text-xs text-muted-foreground">{{ t('personas.activity.noSteps') }}</p>
            <ol class="flex flex-col">
              <li v-for="action in details.actions" :key="action.step" class="flex gap-3 border-b border-border/60 py-2.5 last:border-b-0">
                <component :is="ACTION_ICONS[action.detail.kind]" class="mt-0.5 size-4 shrink-0 text-muted-foreground" aria-hidden="true" />
                <div class="flex min-w-0 flex-1 flex-col gap-1 text-xs">
                  <div class="flex min-w-0 items-center gap-2">
                    <span class="min-w-0 truncate font-medium text-foreground">{{ stepTitle(action) }}</span>
                    <Badge variant="outline" class="ml-auto shrink-0 gap-1.5 font-normal">
                      <span :class="cn('size-1.5 rounded-full', TONE_DOT[actionTone(action.status)])" aria-hidden="true" />{{ t(`personas.step.${action.status}`) }}
                    </Badge>
                  </div>
                  <span class="text-muted-foreground">
                    {{ t('personas.step.line', { line: action.line }) }}<template v-if="action.durationMilliseconds !== null"> · {{ formatDuration(action.durationMilliseconds, locale) }}</template>
                  </span>
                  <code v-if="action.detail.kind === 'branch'" class="font-mono break-words text-muted-foreground">if ({{ action.detail.condition }})</code>
                  <template v-else-if="action.detail.kind === 'open'">
                    <span class="text-muted-foreground">
                      {{ action.detail.images.length
                        ? t('personas.step.images', { loaded: action.detail.images.filter(responded).length, total: action.detail.images.length })
                        : t('personas.step.noImages') }}
                    </span>
                    <span v-if="action.detail.embeddedImages" class="text-muted-foreground">{{ t('personas.step.embeddedImages', { count: action.detail.embeddedImages }) }}</span>
                    <span v-if="action.detail.omittedImages" class="text-muted-foreground">{{ t('personas.step.omittedImages', { count: action.detail.omittedImages }) }}</span>
                    <span v-if="action.detail.rejectedImages" class="text-muted-foreground">{{ t('personas.step.rejectedImages', { count: action.detail.rejectedImages }) }}</span>
                    <ul v-if="action.detail.images.length" class="flex flex-col gap-1">
                      <li v-for="(request, index) in action.detail.images" :key="index" class="flex min-w-0 items-start gap-2">
                        <span :class="cn('mt-1 size-1.5 shrink-0 rounded-full', TONE_DOT[requestTone(request)])" aria-hidden="true" />
                        <span class="flex min-w-0 flex-col">
                          <span class="truncate font-mono" :title="request.url">{{ displayUrl(request.url) }}</span>
                          <span class="text-muted-foreground">{{ requestLine(request) }}</span>
                        </span>
                      </li>
                    </ul>
                  </template>
                  <template v-else-if="action.detail.kind === 'click'">
                    <span v-if="action.detail.text" class="truncate text-muted-foreground">{{ t('personas.step.selectedLink', { text: action.detail.text }) }}</span>
                    <span v-if="action.detail.url" class="truncate font-mono" :title="action.detail.url">{{ displayUrl(action.detail.url) }}</span>
                    <span v-if="action.detail.request" class="flex min-w-0 items-start gap-2">
                      <span :class="cn('mt-1 size-1.5 shrink-0 rounded-full', TONE_DOT[requestTone(action.detail.request)])" aria-hidden="true" />
                      <span class="flex min-w-0 flex-col">
                        <span class="text-muted-foreground">{{ requestLine(action.detail.request) }}</span>
                        <span v-if="action.detail.request.finalUrl" class="truncate font-mono text-muted-foreground" :title="action.detail.request.finalUrl">→ {{ displayUrl(action.detail.request.finalUrl) }}</span>
                      </span>
                    </span>
                  </template>
                  <p v-if="action.error" class="break-words text-destructive">{{ action.error }}</p>
                </div>
              </li>
            </ol>
          </section>
        </template>

        <template v-else>
          <div v-if="loading" class="flex flex-col" role="status">
            <div v-for="index in 5" :key="index" class="flex flex-col gap-2 border-b px-4 py-3"><Skeleton class="h-3.5 w-2/3" /><Skeleton class="h-3 w-1/3" /></div>
          </div>
          <Empty v-else-if="!executions.length && !error" class="border-0">
            <EmptyHeader>
              <EmptyTitle>{{ t('personas.activity.title', { name: props.persona?.name ?? '' }) }}</EmptyTitle>
              <EmptyDescription>{{ t('personas.activity.empty', { address: props.persona?.address ?? '' }) }}</EmptyDescription>
            </EmptyHeader>
          </Empty>
          <ul v-else class="flex flex-col">
            <li v-for="execution in executions" :key="execution.id">
              <button
                type="button"
                class="flex w-full min-w-0 items-start gap-3 border-b px-4 py-3 text-left transition-colors hover:bg-accent/60 focus-visible:bg-accent/60 focus-visible:outline-none disabled:opacity-60"
                :disabled="detailsLoading"
                @click="showDetails(execution)"
              >
                <span :class="cn('mt-1.5 size-2 shrink-0 rounded-full', TONE_DOT[executionTone(execution.status)])" aria-hidden="true" />
                <span class="flex min-w-0 flex-1 flex-col gap-0.5">
                  <span class="truncate text-sm">{{ execution.subject || t('personas.activity.noSubject') }}</span>
                  <span class="text-xs text-muted-foreground">
                    {{ t(`personas.status.${execution.status}`) }} · <time :title="formatFullDate(execution.scheduledAt * 1000)">{{ t('personas.activity.scheduled', { time: stamp(execution.scheduledAt) }) }}</time>
                  </span>
                  <span v-if="execution.error" class="truncate text-xs text-destructive" :title="execution.error">{{ execution.error }}</span>
                </span>
                <ChevronRightIcon class="mt-1 size-4 shrink-0 text-muted-foreground" aria-hidden="true" />
              </button>
            </li>
          </ul>
          <div v-if="hasOlder" class="flex justify-center p-3">
            <Button variant="outline" size="sm" :disabled="loadingOlder" @click="loadOlder">
              <Spinner v-if="loadingOlder" data-icon="inline-start" aria-hidden="true" />{{ t('personas.activity.loadOlder') }}
            </Button>
          </div>
        </template>
      </div>
      <div v-if="unfinished" class="flex items-center gap-2 border-t px-4 py-2 text-xs text-muted-foreground" role="status">
        <RefreshCwIcon class="size-3.5 animate-spin" aria-hidden="true" />{{ t('personas.status.running') }}
      </div>
    </SheetContent>
  </Sheet>
</template>
