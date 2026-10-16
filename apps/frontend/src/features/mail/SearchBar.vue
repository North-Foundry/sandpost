<script setup lang="ts">
import { useI18n } from 'vue-i18n'
import { computed, nextTick, onBeforeUnmount, ref, useTemplateRef, watch } from 'vue'
import { onClickOutside, useEventListener } from '@vueuse/core'
import { CornerDownLeftIcon, PaperclipIcon, SearchIcon, TriangleAlertIcon, XIcon } from '@lucide/vue'
import type { MessageSummary, QueryDiagnostic } from '@/api/contracts'
import { characterIndexForByteOffset } from '@/api/admin'
import { useSandpostClient } from '@/runtime/client'
import { asSentence, describeError } from '@/utils/errors'
import { formatListDate, sendersLabel } from '@/utils/format'
import { cn } from '@/lib/utils'
import { Badge } from '@/components/ui/badge'
import { InputGroup, InputGroupAddon, InputGroupButton } from '@/components/ui/input-group'
import { Kbd } from '@/components/ui/kbd'
import { Spinner } from '@/components/ui/spinner'
import QueryHighlight from '@/features/query/QueryHighlight.vue'
import { applyShortcut, interpretSearch, SEARCH_SHORTCUTS, suggestShortcuts, tokenAtCaret, type SearchShortcut } from './searchSyntax'

const PREVIEW_LIMIT = 8
const { t, locale } = useI18n()

const query = defineModel<string>('query', { required: true })
const emit = defineEmits<{
  /**
   * Open one matching message.
   */
  (event: 'select', message: MessageSummary): void

  /**
   * Show every match as the paginated message list.
   */
  (event: 'submit', input: string): void

  /**
   * The field was emptied by the user.
   */
  (event: 'clear'): void
}>()
const client = useSandpostClient()
const input = useTemplateRef<HTMLInputElement>('input')
const container = useTemplateRef<HTMLElement>('container')
const open = ref(false)
const active = ref(-1)
const caret = ref(0)
const results = ref<MessageSummary[]>([])
const loading = ref(false)
const error = ref('')
const diagnostic = ref<QueryDiagnostic | null>(null)
const shortcutLabel = typeof navigator !== 'undefined' && /Mac|iPhone|iPad/.test(navigator.platform) ? '⌘K' : 'Ctrl K'
let timer: ReturnType<typeof setTimeout> | null = null
let request = 0

const interpretation = computed(() => { void locale.value; return interpretSearch(query.value) })
const searchable = computed(() => 'query' in interpretation.value ? interpretation.value.query : null)
const token = computed(() => tokenAtCaret(query.value, caret.value))
const suggestions = computed(() => token.value ? suggestShortcuts(token.value.text) : [])

/**
 * The canonical error rendered as the offending excerpt with a caret under the reported position.
 */
const diagnosticCaret = computed(() => {
  if (!diagnostic.value) return null
  const source = query.value.trim()
  const index = characterIndexForByteOffset(source, diagnostic.value.position)
  return { before: source.slice(0, index), after: source.slice(index) }
})

/**
 * Results plus the trailing "show all" option form one keyboard-navigable list.
 */
const optionCount = computed(() => searchable.value && !diagnostic.value ? results.value.length + 1 : 0)

/**
 * Fetch a short preview of matches, validating canonical input first so errors point at the query.
 */
function schedulePreview(): void {
  const current = ++request
  if (timer) clearTimeout(timer)
  results.value = []
  error.value = ''
  diagnostic.value = null
  const interpreted = interpretation.value
  if (!('query' in interpreted)) { loading.value = false; return }
  loading.value = true
  timer = setTimeout(async () => {
    try {
      if (interpreted.kind === 'canonical') {
        const validation = await client.admin.validateQuery(interpreted.query)
        if (current !== request) return
        if (!validation.valid) { diagnostic.value = validation.error; return }
      }
      const found = await client.messages.list({ query: interpreted.query, limit: PREVIEW_LIMIT })
      if (current === request) results.value = found
    } catch (failure) {
      if (current === request) error.value = describeError(failure, t('mail.search.unavailable'))
    } finally {
      if (current === request) loading.value = false
    }
  }, 150)
}

/**
 * Track the caret so shortcut suggestions follow the token being typed.
 */
function syncCaret(): void {
  caret.value = input.value?.selectionStart ?? query.value.length
}

/**
 * Insert a shortcut at the caret token, or append it when nothing is being typed.
 */
function insertShortcut(shortcut: SearchShortcut): void {
  const target = token.value ?? { start: query.value.length, end: query.value.length, text: '' }
  const spaced = target.text || !query.value || query.value.endsWith(' ') ? query.value : `${query.value} `
  const adjusted = target.text ? target : { start: spaced.length, end: spaced.length, text: '' }
  const next = applyShortcut(spaced, adjusted, shortcut)
  query.value = next.value
  open.value = true
  void nextTick(() => {
    input.value?.focus()
    input.value?.setSelectionRange(next.caret, next.caret)
    syncCaret()
  })
}

/**
 * Navigate results, accept completions, submit, or dismiss without leaving the field.
 */
function onKeydown(event: KeyboardEvent): void {
  if (event.key === 'Escape') {
    if (open.value) { open.value = false; event.preventDefault() } else if (query.value) { clear() } else input.value?.blur()
    return
  }
  if (event.key === 'Tab' && open.value && suggestions.value.length && !event.shiftKey) {
    event.preventDefault()
    insertShortcut(suggestions.value[0]!)
    return
  }
  if ((event.key === 'ArrowDown' || event.key === 'ArrowUp') && optionCount.value) {
    event.preventDefault()
    open.value = true
    const step = event.key === 'ArrowDown' ? 1 : -1
    active.value = active.value < 0 ? (step > 0 ? 0 : optionCount.value - 1) : (active.value + step + optionCount.value) % optionCount.value
    void nextTick(() => container.value?.querySelector('[aria-selected="true"]')?.scrollIntoView({ block: 'nearest' }))
    return
  }
  if (event.key === 'Enter' && searchable.value && !diagnostic.value) {
    event.preventDefault()
    const message = active.value >= 0 ? results.value[active.value] : undefined
    if (message) selectResult(message)
    else submit()
  }
}

/**
 * Open one message and close the preview.
 */
function selectResult(message: MessageSummary): void {
  open.value = false
  input.value?.blur()
  emit('select', message)
}

/**
 * Apply the query to the message list.
 */
function submit(): void {
  open.value = false
  input.value?.blur()
  emit('submit', query.value.trim())
}

/**
 * Clear the query and keep focus in the field.
 */
function clear(): void {
  query.value = ''
  emit('clear')
  input.value?.focus()
}

/**
 * Focus and select the query, as from a keyboard shortcut.
 */
function focusSearch(): void {
  input.value?.focus()
  input.value?.select()
  open.value = true
}

useEventListener(window, 'keydown', (event: KeyboardEvent) => {
  const target = event.target as HTMLElement | null
  const editing = target?.isContentEditable || ['INPUT', 'TEXTAREA', 'SELECT'].includes(target?.tagName ?? '')
  if ((event.metaKey || event.ctrlKey) && event.key.toLowerCase() === 'k') { event.preventDefault(); focusSearch() }
  else if (event.key === '/' && !editing && !event.metaKey && !event.ctrlKey && !event.altKey) { event.preventDefault(); focusSearch() }
})
onClickOutside(container, () => { open.value = false })
useEventListener(document, 'focusin', (event: FocusEvent) => {
  if (event.target instanceof Node && !container.value?.contains(event.target)) open.value = false
})
watch(query, (value, previous) => {
  active.value = -1
  if (!value.trim() && previous?.trim() && document.activeElement === input.value) emit('clear')
  schedulePreview()
})
watch(results, () => { active.value = -1 })
onBeforeUnmount(() => { request += 1; if (timer) clearTimeout(timer) })

defineExpose({ focus: focusSearch })
</script>

<template>
  <div ref="container" class="relative w-full max-w-2xl">
    <InputGroup class="h-8 bg-muted/60 shadow-none transition-colors has-[input:focus]:bg-background dark:bg-input/30">
      <InputGroupAddon><SearchIcon /></InputGroupAddon>
      <input
        ref="input"
        v-model="query"
        data-slot="input-group-control"
        class="h-full min-w-0 flex-1 bg-transparent px-1.5 text-sm outline-none placeholder:text-muted-foreground [&::-webkit-search-cancel-button]:hidden"
        role="combobox"
        type="search"
        maxlength="1024"
        autocomplete="off"
        autocapitalize="off"
        spellcheck="false"
        :aria-label="t('mail.search.label')"
        aria-autocomplete="list"
        aria-controls="mail-search-panel"
        :aria-expanded="open"
        :aria-activedescendant="open && active >= 0 ? `mail-search-option-${active}` : undefined"
        :placeholder="t('mail.search.placeholder')"
        @input="syncCaret(); open = true"
        @click="syncCaret(); open = true"
        @keyup="syncCaret"
        @focus="open = true"
        @keydown="onKeydown"
      >
      <InputGroupAddon align="inline-end">
        <Spinner v-if="loading" class="text-muted-foreground" aria-hidden="true" />
        <InputGroupButton v-if="query" size="icon-xs" :aria-label="t('mail.search.clear')" @click="clear"><XIcon /></InputGroupButton>
        <Kbd v-else class="max-sm:hidden">{{ shortcutLabel }}</Kbd>
      </InputGroupAddon>
    </InputGroup>

    <div
      v-if="open"
      id="mail-search-panel"
      class="absolute inset-x-0 top-[calc(100%+0.375rem)] z-40 flex max-h-[min(560px,calc(100dvh-4.5rem))] flex-col overflow-hidden rounded-lg bg-popover text-popover-foreground shadow-lg ring-1 ring-foreground/10 animate-in fade-in-0 duration-75"
    >
      <!--
        Shortcut completions for the token at the caret.
      -->
      <div v-if="suggestions.length" class="flex flex-wrap items-center gap-1 border-b px-2.5 py-2" :aria-label="t('mail.search.shortcuts')">
        <button
          v-for="(shortcut, index) in suggestions"
          :key="shortcut.key"
          type="button"
          :class="cn('flex items-center gap-1.5 rounded-md px-1.5 py-0.5 text-xs outline-none hover:bg-accent focus-visible:bg-accent', index === 0 && 'bg-accent')"
          @mousedown.prevent
          @click="insertShortcut(shortcut)"
        >
          <code class="font-mono font-medium">{{ shortcut.insert }}</code>
          <span class="text-muted-foreground">{{ t(shortcut.description) }}</span>
          <Kbd v-if="index === 0" class="h-4 text-[10px]">Tab</Kbd>
        </button>
      </div>

      <!--
        Help shown before typing: the shortcut vocabulary and the canonical alternative.
      -->
      <div v-if="interpretation.kind === 'empty' && !interpretation.pending" class="flex flex-col gap-2 px-3 py-3">
        <span class="text-xs font-medium text-muted-foreground">{{ t('mail.search.by') }}</span>
        <div class="flex flex-wrap gap-1">
          <button
            v-for="shortcut in SEARCH_SHORTCUTS"
            :key="shortcut.key"
            type="button"
            class="rounded-md border px-1.5 py-0.5 font-mono text-xs outline-none hover:bg-accent focus-visible:bg-accent"
            :title="t(shortcut.description)"
            @mousedown.prevent
            @click="insertShortcut(shortcut)"
          >{{ shortcut.insert }}</button>
        </div>
        <p class="text-xs text-muted-foreground">
          {{ t('mail.search.plainWords') }}
          <code class="font-mono"><QueryHighlight source='from.domain == "example.com" and size > 1024' /></code>
        </p>
      </div>
      <p v-else-if="interpretation.kind === 'empty'" class="px-3 py-3 text-xs text-muted-foreground">
        {{ t('mail.search.typeValue', { key: interpretation.pending }) }}
      </p>
      <p v-else-if="interpretation.kind === 'invalid'" class="flex items-center gap-1.5 px-3 py-3 text-xs text-destructive" role="alert">
        <TriangleAlertIcon class="size-3.5 shrink-0" aria-hidden="true" />{{ interpretation.message }}
      </p>

      <template v-else>
        <!--
          What the search means, as understood by the parser.
        -->
        <div class="flex min-w-0 items-center gap-1.5 border-b px-3 py-1.5 text-xs text-muted-foreground">
          <template v-if="interpretation.kind === 'shortcuts'">
            <Badge v-for="(term, index) in interpretation.terms" :key="index" variant="secondary" class="max-w-48 font-normal">
              <span class="truncate"><span v-if="term.negated">{{ t('mail.search.negation') }}</span><span class="text-muted-foreground">{{ term.key }}:</span>{{ term.value }}</span>
            </Badge>
            <span v-if="interpretation.text" class="truncate">“{{ interpretation.text }}”</span>
          </template>
          <span v-else-if="interpretation.kind === 'text'" class="truncate">{{ t('mail.search.textAnywhere') }}</span>
          <code v-else class="min-w-0 truncate font-mono"><QueryHighlight :source="interpretation.query" /></code>
          <span class="ml-auto shrink-0 pl-2 max-sm:hidden">{{ t(interpretation.kind === 'canonical' ? 'mail.search.query' : 'mail.search.search') }} {{ t('mail.search.acrossAll') }}</span>
        </div>

        <div v-if="diagnostic && diagnosticCaret" class="flex flex-col gap-1.5 px-3 py-3" role="alert">
          <span class="flex items-center gap-1.5 text-xs text-destructive"><TriangleAlertIcon class="size-3.5 shrink-0" aria-hidden="true" />{{ asSentence(diagnostic.message) }}</span>
          <code class="block overflow-x-auto font-mono text-xs whitespace-pre text-muted-foreground">{{ diagnosticCaret.before }}<span class="border-b-2 border-destructive text-foreground">{{ diagnosticCaret.after.slice(0, 1) || ' ' }}</span>{{ diagnosticCaret.after.slice(1) }}</code>
          <span class="text-xs text-muted-foreground">{{ t('mail.search.quoteTip') }}</span>
        </div>
        <p v-else-if="error" class="px-3 py-4 text-center text-xs text-destructive" role="alert">{{ error }}</p>
        <div v-else class="min-h-0 overflow-y-auto p-1" role="listbox" :aria-label="t('mail.search.matchingMessages')" :aria-busy="loading">
          <p v-if="loading && !results.length" class="px-2 py-3 text-center text-xs text-muted-foreground" role="status">{{ t('mail.search.searching') }}</p>
          <p v-else-if="!results.length" class="px-2 py-3 text-center text-xs text-muted-foreground">{{ t('mail.search.noMatches') }}</p>
          <button
            v-for="(message, index) in results"
            :id="`mail-search-option-${index}`"
            :key="message.id"
            type="button"
            role="option"
            :aria-selected="active === index"
            :class="cn('grid w-full grid-cols-[minmax(0,10rem)_minmax(0,1fr)_auto] items-baseline gap-3 rounded-md px-2 py-1.5 text-left outline-none', active === index && 'bg-accent text-accent-foreground')"
            @pointermove="active = index"
            @click="selectResult(message)"
          >
            <span class="truncate font-medium">{{ sendersLabel(message.from) }}</span>
            <span class="truncate">
              {{ message.subject || t('mail.detail.noSubject') }}
              <span v-if="message.preview" class="text-muted-foreground"> — {{ message.preview }}</span>
            </span>
            <span class="flex items-center gap-1.5 text-xs text-muted-foreground tabular-nums">
              <PaperclipIcon v-if="message.attachmentCount" class="size-3" :aria-label="t('mail.search.hasAttachments')" />
              {{ formatListDate(message.receivedAt) }}
            </span>
          </button>
        </div>
        <button
          v-if="!diagnostic && !error"
          :id="`mail-search-option-${results.length}`"
          type="button"
          role="option"
          :aria-selected="active === results.length"
          :class="cn('flex w-full items-center justify-between gap-2 border-t px-3 py-2 text-left text-xs text-muted-foreground outline-none hover:bg-accent', active === results.length && 'bg-accent text-accent-foreground')"
          @pointermove="active = results.length"
          @click="submit"
        >
          <span>{{ t('mail.search.showAll') }}</span>
          <span class="flex items-center gap-1"><Kbd><CornerDownLeftIcon /></Kbd></span>
        </button>
      </template>
    </div>
  </div>
</template>
