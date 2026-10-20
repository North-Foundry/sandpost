<script setup lang="ts">
import { computed, onBeforeUnmount, onMounted, ref, useTemplateRef, watch } from 'vue'
import { useI18n } from 'vue-i18n'
import { BookOpenIcon, ChevronDownIcon, CircleAlertIcon, CircleCheckIcon } from '@lucide/vue'
import type { PersonaScriptValidation } from '@/api/contracts'
import { useSandpostClient } from '@/runtime/client'
import { asSentence, describeError } from '@/utils/errors'
import { cn } from '@/lib/utils'
import { Button } from '@/components/ui/button'
import { Collapsible, CollapsibleContent, CollapsibleTrigger } from '@/components/ui/collapsible'
import { Spinner } from '@/components/ui/spinner'
import { formatDuration } from './format'
import { createScriptEditor, scriptDiagnostics, type ScriptEditorHandle } from './scriptEditor'

const props = defineProps<{
  id: string
  disabled?: boolean
  describedBy?: string
}>()
const source = defineModel<string>({ required: true })

/**
 * `null` while a validation request is pending.
 */
const valid = defineModel<boolean | null>('valid', { default: null })
const { t, locale } = useI18n()
const client = useSandpostClient()
const host = useTemplateRef<HTMLElement>('host')
const validation = ref<PersonaScriptValidation | null>(null)
const failure = ref('')
const referenceOpen = ref(false)
const cursor = ref({ line: 1, column: 1 })
let editor: ScriptEditorHandle | null = null
let label: HTMLLabelElement | null = null
const labelId = ref('')
let timer: ReturnType<typeof setTimeout> | null = null
let request = 0

/**
 * Script syntax is not translated, so the example is the same in every language.
 */
const PLACEHOLDER = 'wait 5s;\nopen;\nif (subject contains "Verify") {\n    click link("a.verify");\n}'

const REFERENCE = computed(() => [
  ['open;', t('personas.reference.open')],
  ['wait 500ms; wait 5s; wait 2m;', t('personas.reference.wait')],
  ['click link("a.verify");', t('personas.reference.click')],
  ['click link(text: "Confirm*");', t('personas.reference.clickText')],
  ['click link(href: "*unsubscribe*");', t('personas.reference.clickHref')],
  ['if (from.domain == "example.com") { … } else { … }', t('personas.reference.condition')],
] as const)

const statusError = computed(() => failure.value || (validation.value?.error ? asSentence(validation.value.error.message) : t('personas.script.invalid')))
const summary = computed(() => {
  const result = validation.value
  if (!result?.valid || result.statements === null) return t('personas.script.valid')
  return t('personas.script.summary', {
    statements: result.statements,
    wait: formatDuration(result.longestWaitMilliseconds ?? 0, locale.value),
  })
})

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
 * Validate on the server after typing pauses, then mark the first problem inline.
 */
function scheduleValidation(): void {
  const current = ++request
  valid.value = null
  if (timer) clearTimeout(timer)
  timer = setTimeout(async () => {
    const checked = source.value
    try {
      const result = await client.personas!.validate(checked)
      if (current !== request) return
      validation.value = result
      failure.value = ''
      valid.value = result.valid
      editor?.showDiagnostics(scriptDiagnostics(checked, result.error))
    } catch (error) {
      if (current !== request) return
      failure.value = describeError(error, t('personas.script.checkFailed'))
      valid.value = false
      editor?.showDiagnostics([])
    }
  }, 250)
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
  editor = createScriptEditor(host.value!, {
    doc: source.value,
    placeholder: PLACEHOLDER,
    disabled: props.disabled,
    attributes: contentAttributes.value,

    /**
     * Publish editor changes through the model.
     */
    onChange: (doc) => { source.value = doc },

    /**
     * Show the cursor position in the status bar.
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
        valid === false && 'border-destructive ring-3 ring-destructive/20 focus-within:border-destructive focus-within:ring-destructive/20 dark:border-destructive/50',
        props.disabled && 'cursor-not-allowed bg-input/50 opacity-50 dark:bg-input/80',
      )"
    >
      <div ref="host" class="min-w-0" />
      <div class="flex h-7 min-w-0 items-center gap-3 rounded-b-[inherit] border-t bg-muted/50 px-2.5 text-xs text-muted-foreground">
        <div :id="`${props.id}-status`" class="flex min-w-0 items-center gap-1.5" aria-live="polite">
          <template v-if="valid === null"><Spinner class="size-3" aria-hidden="true" /><span>{{ t('personas.script.checking') }}</span></template>
          <template v-else-if="valid"><CircleCheckIcon class="size-3.5 shrink-0 text-success" aria-hidden="true" /><span class="truncate">{{ summary }}</span></template>
          <template v-else>
            <CircleAlertIcon class="size-3.5 shrink-0 text-destructive" aria-hidden="true" />
            <span class="truncate text-destructive" :title="statusError">
              <template v-if="validation?.error && !failure">{{ t('personas.step.line', { line: validation.error.line }) }} · </template>{{ statusError }}
            </span>
          </template>
        </div>
        <span class="ml-auto shrink-0 font-mono text-[11px] tabular-nums">{{ t('query.status.position', cursor) }}</span>
      </div>
    </div>
    <Collapsible v-model:open="referenceOpen">
      <CollapsibleTrigger as-child>
        <Button variant="ghost" size="sm" class="-ml-2 w-fit text-muted-foreground">
          <BookOpenIcon data-icon="inline-start" />{{ t('personas.reference.title') }}<ChevronDownIcon data-icon="inline-end" :class="referenceOpen && 'rotate-180'" />
        </Button>
      </CollapsibleTrigger>
      <CollapsibleContent>
        <div class="mt-1 flex flex-col gap-3 rounded-lg border bg-muted/30 p-3 text-xs text-foreground">
          <dl class="flex flex-col gap-2.5">
            <div v-for="[syntax, meaning] in REFERENCE" :key="syntax" class="flex min-w-0 flex-col gap-0.5">
              <dt class="font-mono break-words">{{ syntax }}</dt>
              <dd class="text-muted-foreground">{{ meaning }}</dd>
            </div>
          </dl>
          <p class="border-t pt-2.5 text-muted-foreground">{{ t('personas.reference.limits') }}</p>
        </div>
      </CollapsibleContent>
    </Collapsible>
  </div>
</template>
