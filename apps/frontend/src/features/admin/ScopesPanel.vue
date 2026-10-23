<script setup lang="ts">
import { computed, ref, watch } from 'vue'
import { useI18n } from 'vue-i18n'
import { useRoute, useRouter } from 'vue-router'
import { ArrowDownIcon, ArrowUpIcon, FilterIcon, LayersIcon, PlusIcon, Trash2Icon, UsersIcon } from '@lucide/vue'
import type { Scope, ScopeInput, UserAccount } from '@/api/contracts'
import { useSandpostClient } from '@/runtime/client'
import { describeError } from '@/utils/errors'
import { cn } from '@/lib/utils'
import { Alert, AlertDescription } from '@/components/ui/alert'
import { Badge } from '@/components/ui/badge'
import { Button } from '@/components/ui/button'
import { Card, CardAction, CardContent, CardDescription, CardFooter, CardHeader, CardTitle } from '@/components/ui/card'
import { Checkbox } from '@/components/ui/checkbox'
import { DialogFooter } from '@/components/ui/dialog'
import { Empty, EmptyContent, EmptyDescription, EmptyHeader, EmptyMedia, EmptyTitle } from '@/components/ui/empty'
import { Field, FieldContent, FieldDescription, FieldError, FieldGroup, FieldLabel, FieldLegend, FieldSet } from '@/components/ui/field'
import { Input } from '@/components/ui/input'
import { NativeSelect, NativeSelectOption } from '@/components/ui/native-select'
import { Separator } from '@/components/ui/separator'
import { Spinner } from '@/components/ui/spinner'
import BusyDialog from '@/components/BusyDialog.vue'
import QueryEditor from '@/features/query/QueryEditor.vue'
import ScopePreview from './ScopePreview.vue'
import ScopeRoutingActions from './ScopeRoutingActions.vue'
import { ancestors, flattenScopes, nextPosition, siblings, subtreeIds } from './scopeTree'

/**
 * A scope being edited; `isNew` drafts have not been saved yet.
 */
interface ScopeDraft extends ScopeInput { isNew: boolean }

/**
 * An editor change that waits for the user to discard unsaved edits.
 */
type PendingChange = { kind: 'select'; id: string } | { kind: 'new'; parentId: string | null }

const ROOT = ''
const props = defineProps<{
  scopes: Scope[]
  users: UserAccount[]
  assignments: Record<string, string[]>
}>()
const emit = defineEmits<{
  /**
   * Notify the page that scope definitions or assignments changed.
   */
  (event: 'changed'): void
}>()
const client = useSandpostClient()
const { t } = useI18n()
const route = useRoute()
const router = useRouter()
const draft = ref<ScopeDraft | null>(null)
const filterValid = ref<boolean | null>(null)
const saving = ref(false)
const moving = ref(false)
const error = ref('')
const deleteOpen = ref(false)
const deleting = ref(false)
const deleteError = ref('')
const pending = ref<PendingChange | null>(null)
const assignmentBusy = ref<string | null>(null)
const assignmentError = ref('')

const rows = computed(() => flattenScopes(props.scopes))
const original = computed(() => draft.value && !draft.value.isNew ? props.scopes.find((scope) => scope.id === draft.value!.id) ?? null : null)
const dirty = computed(() => {
  const current = draft.value
  if (!current) return false
  if (current.isNew) return true
  const saved = original.value
  return !saved || saved.name !== current.name || (saved.description ?? '') !== (current.description ?? '')
    || saved.filter !== current.filter || saved.parentId !== current.parentId
})
const inherited = computed(() => draft.value ? ancestors(props.scopes, draft.value.parentId) : [])
const parentOptions = computed(() => {
  const excluded = draft.value && !draft.value.isNew ? subtreeIds(props.scopes, draft.value.id) : new Set<string>()
  return rows.value.filter(({ scope }) => !excluded.has(scope.id))
})
const parentValue = computed({
  /**
   * Expose a root parent as the select's empty value.
   */
  get: () => draft.value?.parentId ?? ROOT,

  /**
   * Translate the select's empty value back to a null root identifier.
   */
  set: (value: string) => { if (draft.value) draft.value.parentId = value || null },
})
const hasChildren = computed(() => Boolean(original.value && props.scopes.some((scope) => scope.parentId === original.value!.id)))
const siblingIndex = computed(() => {
  if (!original.value) return { index: -1, count: 0 }
  const list = siblings(props.scopes, original.value.parentId)
  return { index: list.findIndex((scope) => scope.id === original.value!.id), count: list.length }
})

const assignable = computed(() => props.users)
const assigned = computed(() => new Set(draft.value ? props.assignments[draft.value.id] ?? [] : []))
const canSave = computed(() => Boolean(draft.value?.name.trim()) && filterValid.value === true && dirty.value && !saving.value)

/**
 * Copy a saved scope into the editor and reflect the selection in the URL.
 */
function edit(scope: Scope): void {
  draft.value = { ...scope, isNew: false }
  error.value = ''
  assignmentError.value = ''
  if (route.query.scope !== scope.id) void router.replace({ query: { ...route.query, scope: scope.id } })
}

/**
 * Start a new unsaved scope below an optional parent.
 */
function startNew(parentId: string | null): void {
  draft.value = { id: crypto.randomUUID(), parentId, name: '', description: null, filter: '', position: 0, isNew: true }
  error.value = ''
  assignmentError.value = ''
}

/**
 * Apply an editor change now, or ask first when unsaved edits would be lost.
 */
function request(change: PendingChange): void {
  if (change.kind === 'select' && change.id === draft.value?.id && !draft.value.isNew) return
  if (dirty.value) { pending.value = change; return }
  apply(change)
}

/**
 * Perform a selection or creation change.
 */
function apply(change: PendingChange): void {
  pending.value = null
  if (change.kind === 'new') { startNew(change.parentId); return }
  const scope = props.scopes.find((candidate) => candidate.id === change.id)
  if (scope) edit(scope)
}

/**
 * Restore the saved values of the scope being edited.
 */
function revert(): void {
  if (original.value) edit(original.value)
  else draft.value = null
}

/**
 * Save the draft; a new parent places the scope after its new siblings.
 */
async function save(): Promise<void> {
  const current = draft.value
  if (!current || !canSave.value) return
  saving.value = true
  error.value = ''
  try {
    const moved = current.isNew || original.value?.parentId !== current.parentId
    const saved = await client.admin.scopes.save({
      id: current.id,
      parentId: current.parentId,
      name: current.name.trim(),
      description: current.description?.trim() || null,
      filter: current.filter,
      position: moved ? nextPosition(props.scopes, current.parentId, current.id) : current.position,
    })
    draft.value = { ...saved, isNew: false }
    void router.replace({ query: { ...route.query, scope: saved.id } })
    emit('changed')
  } catch (failure) {
    error.value = describeError(failure, t('admin.scopes.error.save'))
  } finally {
    saving.value = false
  }
}

/**
 * Swap the scope with its neighbour and renumber the siblings that changed.
 */
async function move(offset: -1 | 1): Promise<void> {
  const scope = original.value
  if (!scope || moving.value) return
  const list = siblings(props.scopes, scope.parentId)
  const index = list.findIndex((candidate) => candidate.id === scope.id)
  const target = index + offset
  if (target < 0 || target >= list.length) return
    ;[list[index], list[target]] = [list[target]!, list[index]!]
  moving.value = true
  error.value = ''
  try {
    for (const [position, sibling] of list.entries()) {
      if (sibling.position !== position) await client.admin.scopes.save({ ...sibling, position })
    }
    emit('changed')
  } catch (failure) {
    error.value = describeError(failure, t('admin.scopes.error.move'))
    emit('changed')
  } finally {
    moving.value = false
  }
}

/**
 * Delete the confirmed scope and select its parent.
 */
async function remove(): Promise<void> {
  const scope = original.value
  if (!scope || deleting.value) return
  deleting.value = true
  deleteError.value = ''
  try {
    await client.admin.scopes.delete(scope.id)
    deleteOpen.value = false
    const parent = props.scopes.find((candidate) => candidate.id === scope.parentId)
    draft.value = null
    void router.replace({ query: { ...route.query, scope: parent?.id } })
    emit('changed')
  } catch (failure) {
    deleteError.value = describeError(failure, t('admin.scopes.error.delete'))
  } finally {
    deleting.value = false
  }
}

/**
 * Grant or revoke the scope for one user.
 */
async function toggleAssignment(member: UserAccount, checked: boolean): Promise<void> {
  const scope = original.value
  if (!scope || assignmentBusy.value) return
  assignmentBusy.value = member.id
  assignmentError.value = ''
  try {
    if (checked) await client.admin.scopes.assign(scope.id, member.id)
    else await client.admin.scopes.unassign(scope.id, member.id)
    emit('changed')
  } catch (failure) {
    assignmentError.value = describeError(failure, t('admin.scopes.error.assignment'))
  } finally {
    assignmentBusy.value = null
  }
}

/**
 * Keep the editor on the URL's scope, refreshing clean drafts when definitions reload.
 */
watch(() => [props.scopes, route.query.scope] as const, ([scopes, requested]) => {
  if (draft.value?.isNew || dirty.value) return
  const wanted = typeof requested === 'string' ? scopes.find((scope) => scope.id === requested) : undefined
  const fallback = wanted ?? scopes.find((scope) => scope.id === draft.value?.id) ?? flattenScopes(scopes)[0]?.scope
  if (fallback) edit(fallback)
  else draft.value = null
}, { immediate: true })
</script>

<template>
  <div class="grid items-start gap-4 lg:grid-cols-[17rem_minmax(0,1fr)]">
    <Card class="gap-3 py-3 lg:sticky lg:top-6">
      <CardHeader class="px-3">
        <CardTitle class="text-sm">{{ t('admin.scopes.tree') }}</CardTitle>
        <CardDescription class="text-xs">{{ t('admin.scopes.children') }}</CardDescription>
        <CardAction><Button variant="outline" size="sm" @click="request({ kind: 'new', parentId: null })"><PlusIcon data-icon="inline-start" />{{ t('admin.scopes.root') }}</Button></CardAction>
      </CardHeader>
      <CardContent class="px-2">
        <p v-if="!rows.length" class="px-2 py-4 text-center text-xs text-muted-foreground">{{ t('admin.scopes.none') }}</p>
        <ul v-else class="flex flex-col gap-0.5" :aria-label="t('admin.scopes.title')">
          <li v-for="{ scope, depth } in rows" :key="scope.id" class="group/scope relative">
            <button
              type="button"
              :class="cn(
                'flex w-full min-w-0 flex-col rounded-md py-1.5 pr-8 text-left outline-none hover:bg-accent focus-visible:ring-3 focus-visible:ring-ring/50',
                draft?.id === scope.id && !draft.isNew && 'bg-accent',
              )"
              :style="{ paddingLeft: `${0.5 + depth * 0.875}rem` }"
              :aria-current="draft?.id === scope.id && !draft.isNew ? 'true' : undefined"
              @click="request({ kind: 'select', id: scope.id })"
            >
              <span class="flex min-w-0 items-center gap-1.5">
                <span v-if="depth" class="text-muted-foreground/60" aria-hidden="true">└</span>
                <span class="truncate text-sm font-medium">{{ scope.name }}</span>
                <Badge v-if="(props.assignments[scope.id] ?? []).length" variant="secondary" class="ml-auto tabular-nums" :aria-label="t('admin.scopes.assignedCount', { count: props.assignments[scope.id]!.length })">
                  <UsersIcon data-icon="inline-start" />{{ props.assignments[scope.id]!.length }}
                </Badge>
              </span>
              <span :class="cn('truncate font-mono text-[11px] text-muted-foreground', depth && 'pl-4')">{{ scope.filter.trim() || t('admin.scopes.allMail') }}</span>
            </button>
            <Button
              variant="ghost"
              size="icon-xs"
              class="absolute top-1.5 right-1 opacity-0 group-hover/scope:opacity-100 focus-visible:opacity-100"
              :aria-label="t('admin.scopes.newInside', { name: scope.name })"
              @click="request({ kind: 'new', parentId: scope.id })"
            ><PlusIcon /></Button>
          </li>
        </ul>
      </CardContent>
    </Card>

    <Empty v-if="!draft" class="border">
      <EmptyHeader>
        <EmptyMedia variant="icon"><LayersIcon /></EmptyMedia>
        <EmptyTitle>{{ t('admin.scopes.none') }}</EmptyTitle>
        <EmptyDescription>
          {{ t('admin.scopes.emptyDescription') }}
        </EmptyDescription>
      </EmptyHeader>
      <EmptyContent><Button @click="startNew(null)"><PlusIcon data-icon="inline-start" />{{ t('admin.scopes.createFirst') }}</Button></EmptyContent>
    </Empty>

    <Card v-else>
      <CardHeader>
        <CardTitle>{{ draft.isNew ? t('admin.scopes.newTitle') : original?.name }}</CardTitle>
        <CardDescription>
          {{ inherited.length ? t('admin.scopes.inside', { path: inherited.map((scope) => scope.name).join(' › ') }) : t('admin.scopes.rootScope') }}
        </CardDescription>
        <CardAction v-if="!draft.isNew" class="flex gap-1">
          <Button variant="ghost" size="icon-sm" :aria-label="t('admin.common.moveUp')" :disabled="moving || dirty || siblingIndex.index <= 0" @click="move(-1)"><ArrowUpIcon /></Button>
          <Button variant="ghost" size="icon-sm" :aria-label="t('admin.common.moveDown')" :disabled="moving || dirty || siblingIndex.index < 0 || siblingIndex.index >= siblingIndex.count - 1" @click="move(1)"><ArrowDownIcon /></Button>
        </CardAction>
      </CardHeader>
      <CardContent>
        <form id="scope-form" class="flex flex-col gap-6" @submit.prevent="save">
          <FieldGroup class="gap-4">
            <div class="grid gap-4 md:grid-cols-2">
              <Field :data-disabled="saving">
                <FieldLabel for="scope-name">{{ t('admin.scopes.name') }}</FieldLabel>
                <Input id="scope-name" v-model="draft.name" :placeholder="t('admin.scopes.nameExample')" required maxlength="100" :disabled="saving" />
              </Field>
              <Field :data-disabled="saving">
                <FieldLabel for="scope-parent">{{ t('admin.scopes.parent') }}</FieldLabel>
                <NativeSelect id="scope-parent" v-model="parentValue" class="w-full" :disabled="saving">
                  <NativeSelectOption :value="ROOT">{{ t('admin.scopes.rootParent') }}</NativeSelectOption>
                  <NativeSelectOption v-for="{ scope, depth } in parentOptions" :key="scope.id" :value="scope.id">{{ ' '.repeat(depth) }}{{ scope.name }}</NativeSelectOption>
                </NativeSelect>
              </Field>
            </div>
            <Field :data-disabled="saving">
              <FieldLabel for="scope-description">{{ t('admin.scopes.descriptionLabel') }} <span class="font-normal text-muted-foreground">{{ t('admin.scopes.optional') }}</span></FieldLabel>
              <Input id="scope-description" :model-value="draft.description ?? ''" :placeholder="t('admin.scopes.descriptionPlaceholder')" maxlength="500" :disabled="saving" @update:model-value="draft.description = String($event)" />
            </Field>

            <div v-if="inherited.length" class="flex flex-col gap-2">
              <span class="text-sm font-medium">{{ t('admin.scopes.inheritedFilters') }}</span>
              <ol class="flex flex-col overflow-hidden rounded-lg border bg-muted/30 text-xs">
                <li v-for="(ancestor, index) in inherited" :key="ancestor.id" class="flex min-w-0 items-baseline gap-2 border-b px-3 py-1.5 last:border-b-0">
                  <span class="shrink-0 font-medium">{{ ancestor.name }}</span>
                  <code class="min-w-0 flex-1 truncate font-mono text-muted-foreground">{{ ancestor.filter.trim() || t('admin.scopes.allMail') }}</code>
                  <span v-if="index < inherited.length - 1" class="shrink-0 text-muted-foreground">{{ t('admin.scopes.and') }}</span>
                </li>
              </ol>
              <FieldDescription>{{ t('admin.scopes.inheritedDescription') }}</FieldDescription>
            </div>

            <Field :data-disabled="saving" :data-invalid="filterValid === false">
              <FieldLabel for="scope-filter"><FilterIcon class="size-3.5" aria-hidden="true" />{{ t('admin.scopes.filterLabel') }}</FieldLabel>
              <QueryEditor id="scope-filter" v-model="draft.filter" v-model:valid="filterValid" :disabled="saving" />
            </Field>
          </FieldGroup>

          <Separator />
          <ScopePreview :parent-id="draft.parentId" :filter="draft.filter" :valid="filterValid" />

          <template v-if="!draft.isNew">
            <Separator />
            <FieldSet class="gap-3">
              <FieldLegend variant="label">{{ t('admin.scopes.assignedUsers') }}</FieldLegend>
              <FieldDescription class="-mt-1">{{ t('admin.scopes.assignDescription') }}</FieldDescription>
              <p v-if="!assignable.length" class="text-sm text-muted-foreground">{{ t('admin.scopes.noUsers') }}</p>
              <Field v-for="member in assignable" :key="member.id" orientation="horizontal" :data-disabled="Boolean(assignmentBusy) || dirty">
                <Checkbox
                  :id="`assign-${member.id}`"
                  :model-value="assigned.has(member.id)"
                  :disabled="Boolean(assignmentBusy) || dirty"
                  @update:model-value="toggleAssignment(member, $event === true)"
                />
                <FieldContent>
                  <FieldLabel :for="`assign-${member.id}`" class="font-normal">
                    {{ member.name }} <span class="text-muted-foreground">{{ member.email }}</span>
                    <Spinner v-if="assignmentBusy === member.id" class="size-3" aria-hidden="true" />
                  </FieldLabel>
                  <FieldDescription v-if="member.mailAccess === 'all'" class="text-xs">{{ t('admin.scopes.alreadyAll') }}</FieldDescription>
                </FieldContent>
              </Field>
              <FieldDescription v-if="dirty && assignable.length" class="text-xs">{{ t('admin.scopes.saveDiscardBefore') }}</FieldDescription>
              <FieldError v-if="assignmentError" role="alert">{{ assignmentError }}</FieldError>
            </FieldSet>
          </template>
        </form>
        <template v-if="!draft.isNew">
          <Separator class="my-6" />
          <ScopeRoutingActions :scope-id="draft.id" />
        </template>
        <Alert v-if="error" variant="destructive" class="mt-4" role="alert"><AlertDescription>{{ error }}</AlertDescription></Alert>
      </CardContent>
      <CardFooter class="flex flex-wrap gap-2 border-t">
        <Button v-if="!draft.isNew" variant="outline" class="text-destructive" :disabled="saving || hasChildren" :title="hasChildren ? t('admin.scopes.deleteChildren') : undefined" @click="deleteError = ''; deleteOpen = true">
          <Trash2Icon data-icon="inline-start" />{{ t('admin.scopes.delete') }}
        </Button>
        <span v-if="hasChildren" class="text-xs text-muted-foreground">{{ t('admin.scopes.hasChildren') }}</span>
        <div class="ml-auto flex gap-2">
          <Button v-if="dirty" variant="ghost" type="button" :disabled="saving" @click="revert">{{ draft.isNew ? t('admin.scopes.cancel') : t('admin.scopes.discardChanges') }}</Button>
          <Button type="submit" form="scope-form" :disabled="!canSave">
            <Spinner v-if="saving" data-icon="inline-start" aria-hidden="true" />{{ saving ? t('admin.scopes.saving') : draft.isNew ? t('admin.scopes.create') : t('admin.scopes.save') }}
          </Button>
        </div>
      </CardFooter>
    </Card>

    <BusyDialog :open="Boolean(pending)" :title="t('admin.scopes.unsavedTitle')" :description="t('admin.scopes.unsavedDescription')" @update:open="!$event && (pending = null)">
      <DialogFooter>
        <Button variant="outline" @click="pending = null">{{ t('admin.scopes.keepEditing') }}</Button>
        <Button variant="destructive" @click="pending && apply(pending)">{{ t('admin.scopes.discard') }}</Button>
      </DialogFooter>
    </BusyDialog>

    <BusyDialog v-model:open="deleteOpen" :title="t('admin.scopes.deleteTitle', { name: original?.name ?? t('admin.scopes.title') })" :description="t('admin.scopes.deleteDescription')" :busy="deleting">
      <Alert v-if="deleteError" variant="destructive" role="alert"><AlertDescription>{{ deleteError }}</AlertDescription></Alert>
      <DialogFooter>
        <Button variant="outline" :disabled="deleting" @click="deleteOpen = false">{{ t('admin.scopes.cancel') }}</Button>
        <Button variant="destructive" :disabled="deleting" @click="remove">
          <Spinner v-if="deleting" data-icon="inline-start" aria-hidden="true" />{{ deleting ? t('admin.users.deleting') : t('admin.scopes.deleteScope') }}
        </Button>
      </DialogFooter>
    </BusyDialog>
  </div>
</template>
