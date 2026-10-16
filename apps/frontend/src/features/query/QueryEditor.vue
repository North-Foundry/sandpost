<script setup lang="ts">
import { computed, onBeforeUnmount, onMounted, ref, useTemplateRef, watch } from 'vue'
import { useI18n } from 'vue-i18n'
import { BookOpenIcon, ChevronDownIcon, CircleAlertIcon, CircleCheckIcon, InfoIcon, PlusIcon } from '@lucide/vue'
import type { QueryDiagnostic } from '@/api/contracts'
import { useSandpostClient } from '@/runtime/client'
import { asSentence, describeError } from '@/utils/errors'
import { cn } from '@/lib/utils'
import { Alert, AlertDescription } from '@/components/ui/alert'
import { Button } from '@/components/ui/button'
import { Collapsible, CollapsibleContent, CollapsibleTrigger } from '@/components/ui/collapsible'
import { Kbd } from '@/components/ui/kbd'
import { Spinner } from '@/components/ui/spinner'
import QueryHighlight from './QueryHighlight.vue'
import { createQueryEditor, hasAnyInequality, queryDiagnostics, type QueryEditorHandle } from './editor'

const props = defineProps<{
  id: string
  disabled?: boolean
  describedBy?: string

  /**
   * What an empty query means where it is used, such as "matches every message the parent allows".
   */
  emptyHint?: string
}>()
const source = defineModel<string>({ required: true })
const { t } = useI18n()

/**
 * `null` while a validation request is pending.
 */
const valid = defineModel<boolean | null>('valid', { default: null })
const client = useSandpostClient()
const host = useTemplateRef<HTMLElement>('host')
const diagnostic = ref<QueryDiagnostic | null>(null)
const failure = ref('')
const referenceOpen = ref(false)
const cursor = ref({ line: 1, column: 1 })
let editor: QueryEditorHandle | null = null
let label: HTMLLabelElement | null = null
const labelId = ref('')
let timer: ReturnType<typeof setTimeout> | null = null
let request = 0

const FIELDS = computed(() => [
  ['envelope.from', t('query.reference.fields.smtpSender')],
  ['envelope.to', t('query.reference.fields.smtpRecipients')],
  ['from · to · cc', t('query.reference.fields.mailboxes')],
  ['subject · text · html', t('query.reference.fields.messageContent')],
  ['content', t('query.reference.fields.anyContent')],
  ['header["X-Name"]', t('query.reference.fields.headerValues')],
  ['size · attachment_count · received_at', t('query.reference.fields.numbers')],
  ['has_attachments', t('query.reference.fields.boolean')],
] as const)

const OPERATORS = computed(() => [
  ['== != contains starts_with ends_with', t('query.reference.operators.text')],
  ['matches', t('query.reference.operators.glob')],
  ['< <= > >=', t('query.reference.operators.numbers')],
  ['and · or · not · ( )', t('query.reference.operators.combine')],
] as const)

const EXAMPLES = [
  'envelope.to.domain == "example.test"',
  'from.address matches "*@billing.example.test"',
  'subject contains "invoice"',
  'not (to.domain == "internal.test")',
  'header["X-App"] == "payments"',
] as const

const anyWarning = computed(() => hasAnyInequality(source.value))
const emptyHint = computed(() => props.emptyHint ?? t('query.emptyHint'))
const statusError = computed(() => failure.value || (diagnostic.value ? asSentence(diagnostic.value.message) : t('query.status.invalid')))

/**
 * Attributes of the editable element: the field's id, its label, and what describes it.
 */
const contentAttributes = computed((): Record<string, string> => ({
  id: props.id,
  role: 'textbox',
  'aria-multiline': 'true',
  'aria-invalid': String(valid.value === false),
  'aria-describedby': [props.describedBy, `${props.id}-status`].filter(Boolean).join(' '),
  ...(labelId.value ? { 'aria-labelledby': labelId.value } : {}),
  spellcheck: 'false',
  autocapitalize: 'off',
  autocorrect: 'off',
}))

/**
 * Validate the current source on the server after typing pauses, then mark problems inline.
 */
function scheduleValidation(): void {
  const current = ++request
  valid.value = null
  if (timer) clearTimeout(timer)
  timer = setTimeout(async () => {
    const checked = source.value
    try {
      const result = await client.admin.validateQuery(checked)
      if (current !== request) return
      diagnostic.value = result.error
      failure.value = ''
      valid.value = result.valid
      editor?.showDiagnostics(queryDiagnostics(checked, result.error))
    } catch (error) {
      if (current !== request) return
      failure.value = describeError(error, t('query.status.checkFailed'))
      valid.value = false
      editor?.showDiagnostics(queryDiagnostics(checked, null))
    }
  }, 250)
}

/**
 * Append an example, joining it to an existing filter with `and`.
 */
function insertExample(example: string): void {
  source.value = source.value.trim() ? `(${source.value.trim()}) and ${example}` : example
  editor?.view.focus()
}

/**
 * A `<label for>` cannot target the editable element, so clicking it focuses the editor instead.
 */
function focusEditor(): void {
  editor?.view.focus()
}

onMounted(() => {
  label = document.querySelector<HTMLLabelElement>(`label[for="${CSS.escape(props.id)}"]`)
  if (label) {
    label.id ||= `${props.id}-label`
    labelId.value = label.id
    label.addEventListener('click', focusEditor)
  }
  editor = createQueryEditor(host.value!, {
    doc: source.value,
    placeholder: 'envelope.to.domain == "example.test"',
    disabled: props.disabled,
    attributes: contentAttributes.value,

    /**
     * Publish editor document changes through the component's source model.
     */
    onChange: (doc) => { source.value = doc },

    /**
     * Update the displayed cursor position using the editor's one-based coordinates.
     */
    onCursor: (line, column) => { cursor.value = { line, column } },
  })
})

watch(source, (doc) => { editor?.setDoc(doc); scheduleValidation() }, { immediate: true })
watch(() => props.disabled, (disabled) => editor?.setDisabled(disabled))
watch(contentAttributes, (attributes) => editor?.setAttributes(attributes))
onBeforeUnmount(() => {
  request += 1
  if (timer) clearTimeout(timer)
  label?.removeEventListener('click', focusEditor)
  editor?.destroy()
  editor = null
})
</script>

<template>
  <div class="flex flex-col gap-2">
    <div
      :class="cn(
        'flex min-w-0 flex-col rounded-lg border border-input bg-transparent transition-colors focus-within:border-ring focus-within:ring-3 focus-within:ring-ring/50 dark:bg-input/30',
        valid === false && 'border-destructive ring-3 ring-destructive/20 focus-within:border-destructive focus-within:ring-destructive/20 dark:border-destructive/50 dark:ring-destructive/40 dark:focus-within:border-destructive/50 dark:focus-within:ring-destructive/40',
        props.disabled && 'cursor-not-allowed bg-input/50 opacity-50 dark:bg-input/80',
      )"
    >
      <div ref="host" class="min-w-0" />
      <div class="flex h-7 min-w-0 items-center gap-3 rounded-b-[inherit] border-t bg-muted/50 px-2.5 text-xs text-muted-foreground">
        <div :id="`${props.id}-status`" class="flex min-w-0 items-center gap-1.5" aria-live="polite">
          <template v-if="valid === null"><Spinner class="size-3" aria-hidden="true" /><span>{{ t('query.status.checking') }}</span></template>
          <template v-else-if="valid && !source.trim()"><InfoIcon class="size-3.5 shrink-0" aria-hidden="true" /><span class="truncate" :title="emptyHint">{{ emptyHint }}</span></template>
          <template v-else-if="valid"><CircleCheckIcon class="size-3.5 shrink-0 text-success" aria-hidden="true" /><span>{{ t('query.status.valid') }}</span></template>
          <template v-else>
            <CircleAlertIcon class="size-3.5 shrink-0 text-destructive" aria-hidden="true" />
            <span class="truncate text-destructive" :title="statusError">{{ statusError }}</span>
          </template>
        </div>
        <span class="ml-auto shrink-0 font-mono text-[11px] tabular-nums">{{ t('query.status.position', cursor) }}</span>
        <span class="flex shrink-0 items-center gap-1 max-sm:hidden" aria-hidden="true"><Kbd>Ctrl</Kbd><Kbd>Space</Kbd></span>
      </div>
    </div>
    <Alert v-if="anyWarning">
      <InfoIcon />
      <AlertDescription>{{ t('query.warning.collectionInequality') }}</AlertDescription>
    </Alert>

    <Collapsible v-model:open="referenceOpen">
      <CollapsibleTrigger as-child>
        <Button variant="ghost" size="sm" class="-ml-2 w-fit text-muted-foreground">
          <BookOpenIcon data-icon="inline-start" />{{ t('query.reference.title') }}<ChevronDownIcon data-icon="inline-end" :class="referenceOpen && 'rotate-180'" />
        </Button>
      </CollapsibleTrigger>
      <CollapsibleContent>
        <div class="@container mt-1 flex flex-col gap-4 rounded-lg border bg-muted/30 p-3 text-xs text-foreground">
          <section class="flex flex-col gap-2" :aria-label="t('query.reference.fields.title')">
            <span class="font-medium">{{ t('query.reference.fields.title') }}</span>
            <dl class="grid gap-x-6 gap-y-2.5 @lg:grid-cols-2">
              <div v-for="[field, meaning] in FIELDS" :key="field" class="flex min-w-0 flex-col gap-0.5">
                <dt class="font-mono break-words">{{ field }}</dt>
                <dd class="text-muted-foreground">{{ meaning }}</dd>
              </div>
            </dl>
          </section>
          <section class="flex flex-col gap-2 border-t pt-3" :aria-label="t('query.reference.operators.title')">
            <span class="font-medium">{{ t('query.reference.operators.title') }}</span>
            <dl class="grid gap-x-6 gap-y-2.5 @lg:grid-cols-2">
              <div v-for="[operators, meaning] in OPERATORS" :key="operators" class="flex min-w-0 flex-col gap-0.5">
                <dt class="font-mono break-words">{{ operators }}</dt>
                <dd class="text-muted-foreground">{{ meaning }}</dd>
              </div>
            </dl>
          </section>
          <section class="flex flex-col gap-2 border-t pt-3" :aria-label="t('query.reference.examples.title')">
            <span class="font-medium">{{ t('query.reference.examples.title') }}</span>
            <ul class="flex flex-col gap-1">
              <li v-for="example in EXAMPLES" :key="example" class="flex items-start gap-2">
                <code class="min-w-0 flex-1 py-0.5 font-mono break-words"><QueryHighlight :source="example" /></code>
                <Button variant="ghost" size="icon-xs" :aria-label="t('query.reference.examples.add', { example })" :disabled="props.disabled" @click="insertExample(example)"><PlusIcon /></Button>
              </li>
            </ul>
          </section>
        </div>
      </CollapsibleContent>
    </Collapsible>
  </div>
</template>
