<script setup lang="ts">
import { computed, ref, watch } from 'vue'
import { useI18n } from 'vue-i18n'
import { RouterLink } from 'vue-router'
import { ForwardIcon, PencilIcon, PlusIcon, SendIcon, Trash2Icon, TriangleAlertIcon } from '@lucide/vue'
import type { RoutingAction, RoutingActionType, SmtpOutbound } from '@/api/contracts'
import { useSandpostClient } from '@/runtime/client'
import { describeError } from '@/utils/errors'
import { Alert, AlertDescription } from '@/components/ui/alert'
import { Badge } from '@/components/ui/badge'
import { Button } from '@/components/ui/button'
import { DialogFooter } from '@/components/ui/dialog'
import { Field, FieldContent, FieldDescription, FieldError, FieldGroup, FieldLabel, FieldLegend, FieldSet } from '@/components/ui/field'
import { NativeSelect, NativeSelectOption } from '@/components/ui/native-select'
import { Skeleton } from '@/components/ui/skeleton'
import { Spinner } from '@/components/ui/spinner'
import { Switch } from '@/components/ui/switch'
import { Textarea } from '@/components/ui/textarea'
import BusyDialog from '@/components/BusyDialog.vue'
import {
  MAXIMUM_FORWARD_RECIPIENTS, allowlistCoverage, forwardRecipientIssues, formatRecipientList, hasForwardRecipientIssues, parseRecipientList,
} from '@/features/outbound/recipients'

/**
 * Routing actions of one saved scope: relay to the original recipients or forward a copy, each
 * through an existing SMTP Outbound. Actions apply only to mail whose most specific matching
 * scope is this one; child scopes do not inherit them.
 */
const props = defineProps<{ scopeId: string; disabled?: boolean }>()
const client = useSandpostClient()
const { t } = useI18n()

const actions = ref<RoutingAction[]>([])
const outbounds = ref<SmtpOutbound[]>([])
const loading = ref(true)
const error = ref('')
const rowError = ref('')
const toggling = ref<string | null>(null)

/**
 * The action being created or edited; `id` is null for a new action.
 */
const editor = ref<{ id: string | null; actionType: RoutingActionType; smtpOutboundId: string; recipients: string; enabled: boolean } | null>(null)
const submitted = ref(false)
const saving = ref(false)
const saveError = ref('')
const deleteTarget = ref<RoutingAction | null>(null)
const deleting = ref(false)
const deleteError = ref('')
let generation = 0

const outboundsById = computed(() => new Map(outbounds.value.map((outbound) => [outbound.id, outbound])))
const editorOutbound = computed(() => editor.value ? outboundsById.value.get(editor.value.smtpOutboundId) ?? null : null)
const editorRecipients = computed(() => editor.value ? parseRecipientList(editor.value.recipients) : [])
const recipientIssues = computed(() => forwardRecipientIssues(editorRecipients.value, editorOutbound.value?.allowedRecipients ?? []))
const editorValid = computed(() => Boolean(editor.value && editorOutbound.value)
  && (editor.value!.actionType === 'relay' || !hasForwardRecipientIssues(recipientIssues.value)))

/**
 * Explain the first forward recipient problem, once the user tried to save.
 */
const recipientProblem = computed(() => {
  if (!submitted.value || editor.value?.actionType !== 'forward') return ''
  const issues = recipientIssues.value
  if (issues.empty) return t('outbound.routing.recipientsRequired')
  if (issues.tooMany) return t('outbound.routing.tooManyRecipients', { maximum: MAXIMUM_FORWARD_RECIPIENTS })
  if (issues.invalid.length) return t('outbound.routing.invalidRecipients', { addresses: issues.invalid.join(', ') })
  if (issues.refused.length) return t('outbound.routing.refusedRecipients', { addresses: issues.refused.join(', '), name: editorOutbound.value?.name ?? '' })
  return ''
})

/**
 * Load the scope's actions and the outbounds they can use, ignoring responses for a previous scope.
 */
async function load(): Promise<void> {
  const request = ++generation
  loading.value = true
  error.value = ''
  try {
    const [loadedActions, loadedOutbounds] = await Promise.all([client.admin.routing.list(props.scopeId), client.admin.outbound.list()])
    if (request !== generation) return
    actions.value = loadedActions
    outbounds.value = loadedOutbounds
  } catch (failure) {
    if (request === generation) error.value = describeError(failure, t('outbound.routing.loadError'))
  } finally {
    if (request === generation) loading.value = false
  }
}

/**
 * Name the outbound an action uses, even when it is missing from the loaded list.
 */
function outboundName(action: RoutingAction): string {
  return outboundsById.value.get(action.smtpOutboundId)?.name ?? t('outbound.routing.unknownOutbound')
}

/**
 * Start a new action of the chosen type with the first enabled outbound preselected.
 */
function startNew(actionType: RoutingActionType): void {
  const preferred = outbounds.value.find((outbound) => outbound.enabled) ?? outbounds.value[0]
  editor.value = { id: null, actionType, smtpOutboundId: preferred?.id ?? '', recipients: '', enabled: true }
  submitted.value = false
  saveError.value = ''
}

/**
 * Open an existing action in the editor.
 */
function edit(action: RoutingAction): void {
  editor.value = {
    id: action.id, actionType: action.actionType, smtpOutboundId: action.smtpOutboundId,
    recipients: formatRecipientList(action.forwardRecipients), enabled: action.enabled,
  }
  submitted.value = false
  saveError.value = ''
}

/**
 * Create or replace the edited action.
 */
async function save(): Promise<void> {
  const current = editor.value
  submitted.value = true
  if (!current || !editorValid.value || saving.value) return
  saving.value = true
  saveError.value = ''
  try {
    const input = {
      smtpOutboundId: current.smtpOutboundId,
      actionType: current.actionType,
      forwardRecipients: current.actionType === 'forward' ? editorRecipients.value : [],
      enabled: current.enabled,
    }
    const saved = current.id
      ? await client.admin.routing.update(current.id, input)
      : await client.admin.routing.create(props.scopeId, input)
    actions.value = current.id
      ? actions.value.map((action) => action.id === saved.id ? saved : action)
      : [...actions.value, saved]
    editor.value = null
  } catch (failure) {
    saveError.value = describeError(failure, t('outbound.routing.saveError'))
  } finally {
    saving.value = false
  }
}

/**
 * Enable or disable an action in place, keeping its other settings.
 */
async function toggle(action: RoutingAction, enabled: boolean): Promise<void> {
  if (toggling.value) return
  toggling.value = action.id
  rowError.value = ''
  try {
    const saved = await client.admin.routing.update(action.id, {
      smtpOutboundId: action.smtpOutboundId, actionType: action.actionType, forwardRecipients: action.forwardRecipients, enabled,
    })
    actions.value = actions.value.map((candidate) => candidate.id === saved.id ? saved : candidate)
  } catch (failure) {
    rowError.value = describeError(failure, t('outbound.routing.saveError'))
  } finally {
    toggling.value = null
  }
}

/**
 * Delete the confirmed action.
 */
async function remove(): Promise<void> {
  const target = deleteTarget.value
  if (!target || deleting.value) return
  deleting.value = true
  deleteError.value = ''
  try {
    await client.admin.routing.delete(target.id)
    actions.value = actions.value.filter((action) => action.id !== target.id)
    deleteTarget.value = null
  } catch (failure) {
    deleteError.value = describeError(failure, t('outbound.routing.deleteError'))
  } finally {
    deleting.value = false
  }
}

watch(() => props.scopeId, () => {
  editor.value = null
  deleteTarget.value = null
  rowError.value = ''
  void load()
}, { immediate: true })
</script>

<template>
  <FieldSet class="gap-3">
    <FieldLegend variant="label">{{ t('outbound.routing.title') }}</FieldLegend>
    <FieldDescription class="-mt-1">{{ t('outbound.routing.description') }}</FieldDescription>

    <Alert v-if="error" variant="destructive" role="alert">
      <AlertDescription class="flex items-center gap-3"><span class="flex-1">{{ error }}</span><Button variant="outline" size="sm" type="button" @click="load">{{ t('outbound.common.retry') }}</Button></AlertDescription>
    </Alert>
    <Skeleton v-if="loading" class="h-12 w-full" />
    <template v-else-if="!error">
      <p v-if="!outbounds.length" class="text-sm text-muted-foreground">
        {{ t('outbound.routing.noOutbounds') }}
        <RouterLink :to="{ name: 'admin-smtp-outbound' }" class="font-medium text-foreground underline underline-offset-4">{{ t('outbound.routing.openOutbounds') }}</RouterLink>
      </p>
      <p v-else-if="!actions.length" class="text-sm text-muted-foreground">{{ t('outbound.routing.none') }}</p>
      <ul v-if="actions.length" class="flex flex-col overflow-hidden rounded-lg border">
        <li v-for="action in actions" :key="action.id" class="flex min-w-0 items-center gap-3 border-b px-3 py-2 last:border-b-0">
          <component :is="action.actionType === 'forward' ? ForwardIcon : SendIcon" class="size-4 shrink-0 text-muted-foreground" aria-hidden="true" />
          <span class="flex min-w-0 flex-1 flex-col gap-0.5">
            <span class="flex min-w-0 items-center gap-2">
              <Badge variant="secondary">{{ t(`outbound.action.${action.actionType}`) }}</Badge>
              <span class="truncate text-sm">{{ t('outbound.delivery.via', { name: outboundName(action) }) }}</span>
              <Badge v-if="outboundsById.get(action.smtpOutboundId)?.enabled === false" variant="outline" class="font-normal">{{ t('outbound.routing.outboundDisabled') }}</Badge>
            </span>
            <span v-if="action.actionType === 'forward'" class="truncate font-mono text-xs text-muted-foreground" :title="action.forwardRecipients.join('\n')">{{ action.forwardRecipients.join(', ') }}</span>
            <span v-else class="text-xs text-muted-foreground">{{ t('outbound.routing.relayHint') }}</span>
            <span v-if="allowlistCoverage(outboundsById.get(action.smtpOutboundId)?.allowedRecipients ?? []) === 'nobody'" class="inline-flex items-center gap-1 text-xs text-warning">
              <TriangleAlertIcon class="size-3.5" aria-hidden="true" />{{ t('outbound.routing.outboundDeliversNobody') }}
            </span>
          </span>
          <Switch
            :model-value="action.enabled"
            :disabled="props.disabled || toggling === action.id"
            :aria-label="t('outbound.routing.toggle')"
            @update:model-value="toggle(action, $event)"
          />
          <Button variant="ghost" size="icon-sm" type="button" :disabled="props.disabled" :aria-label="t('outbound.routing.edit')" @click="edit(action)"><PencilIcon /></Button>
          <Button variant="ghost" size="icon-sm" type="button" :disabled="props.disabled" :aria-label="t('outbound.routing.delete')" @click="deleteError = ''; deleteTarget = action"><Trash2Icon /></Button>
        </li>
      </ul>
      <Alert v-if="rowError" variant="destructive" role="alert"><AlertDescription>{{ rowError }}</AlertDescription></Alert>
      <div v-if="outbounds.length" class="flex flex-wrap gap-2">
        <Button variant="outline" size="sm" type="button" :disabled="props.disabled" @click="startNew('relay')"><PlusIcon data-icon="inline-start" />{{ t('outbound.routing.addRelay') }}</Button>
        <Button variant="outline" size="sm" type="button" :disabled="props.disabled" @click="startNew('forward')"><PlusIcon data-icon="inline-start" />{{ t('outbound.routing.addForward') }}</Button>
      </div>
    </template>

    <BusyDialog
      :open="Boolean(editor)"
      :title="editor?.id ? t('outbound.routing.editTitle') : editor?.actionType === 'forward' ? t('outbound.routing.addForward') : t('outbound.routing.addRelay')"
      :description="editor?.actionType === 'forward' ? t('outbound.routing.forwardDescription') : t('outbound.routing.relayDescription')"
      :busy="saving"
      @update:open="!$event && (editor = null)"
    >
      <form v-if="editor" class="flex flex-col gap-4" novalidate @submit.prevent="save">
        <FieldGroup class="gap-4">
          <Field :data-disabled="saving">
            <FieldLabel for="routing-action-type">{{ t('outbound.routing.type') }}</FieldLabel>
            <NativeSelect id="routing-action-type" v-model="editor.actionType" class="w-full" :disabled="saving">
              <NativeSelectOption value="relay">{{ t('outbound.action.relay') }}</NativeSelectOption>
              <NativeSelectOption value="forward">{{ t('outbound.action.forward') }}</NativeSelectOption>
            </NativeSelect>
          </Field>
          <Field :data-disabled="saving">
            <FieldLabel for="routing-action-outbound">{{ t('outbound.routing.outbound') }}</FieldLabel>
            <NativeSelect id="routing-action-outbound" v-model="editor.smtpOutboundId" class="w-full" :disabled="saving">
              <NativeSelectOption v-for="outbound in outbounds" :key="outbound.id" :value="outbound.id">
                {{ outbound.enabled ? outbound.name : t('outbound.routing.disabledOption', { name: outbound.name }) }}
              </NativeSelectOption>
            </NativeSelect>
            <FieldDescription v-if="editorOutbound && allowlistCoverage(editorOutbound.allowedRecipients) === 'nobody'" class="text-xs text-warning">{{ t('outbound.routing.outboundDeliversNobody') }}</FieldDescription>
          </Field>
          <Field v-if="editor.actionType === 'forward'" :data-invalid="Boolean(recipientProblem)" :data-disabled="saving">
            <FieldLabel for="routing-action-recipients">{{ t('outbound.routing.recipients') }}</FieldLabel>
            <Textarea
              id="routing-action-recipients"
              v-model="editor.recipients"
              class="min-h-16 font-mono"
              placeholder="qa@example.com"
              spellcheck="false"
              :aria-invalid="Boolean(recipientProblem)"
              :disabled="saving"
            />
            <FieldDescription class="text-xs">{{ t('outbound.routing.recipientsDescription') }}</FieldDescription>
            <FieldError v-if="recipientProblem">{{ recipientProblem }}</FieldError>
          </Field>
          <Field orientation="horizontal" :data-disabled="saving">
            <Switch id="routing-action-enabled" v-model="editor.enabled" :disabled="saving" />
            <FieldContent>
              <FieldLabel for="routing-action-enabled" class="font-normal">{{ t('outbound.form.enabled') }}</FieldLabel>
            </FieldContent>
          </Field>
        </FieldGroup>
        <Alert v-if="saveError" variant="destructive" role="alert"><AlertDescription>{{ saveError }}</AlertDescription></Alert>
        <DialogFooter>
          <Button variant="outline" type="button" :disabled="saving" @click="editor = null">{{ t('outbound.common.cancel') }}</Button>
          <Button type="submit" :disabled="saving || !editorOutbound || (submitted && !editorValid)">
            <Spinner v-if="saving" data-icon="inline-start" aria-hidden="true" />{{ saving ? t('outbound.common.saving') : t('outbound.common.save') }}
          </Button>
        </DialogFooter>
      </form>
    </BusyDialog>

    <BusyDialog
      :open="Boolean(deleteTarget)"
      :title="t('outbound.routing.deleteTitle')"
      :description="t('outbound.routing.deleteDescription')"
      :busy="deleting"
      @update:open="!$event && (deleteTarget = null)"
    >
      <Alert v-if="deleteError" variant="destructive" role="alert"><AlertDescription>{{ deleteError }}</AlertDescription></Alert>
      <DialogFooter>
        <Button variant="outline" :disabled="deleting" @click="deleteTarget = null">{{ t('outbound.common.cancel') }}</Button>
        <Button variant="destructive" :disabled="deleting" @click="remove">
          <Spinner v-if="deleting" data-icon="inline-start" aria-hidden="true" />{{ deleting ? t('outbound.common.deleting') : t('outbound.routing.delete') }}
        </Button>
      </DialogFooter>
    </BusyDialog>
  </FieldSet>
</template>
