<script setup lang="ts">
import { computed, onBeforeUnmount, onMounted, ref, watch } from 'vue'
import { useI18n } from 'vue-i18n'
import { useRouter } from 'vue-router'
import { ChevronRightIcon, PencilIcon, PlusIcon, SearchIcon, Trash2Icon, UsersIcon } from '@lucide/vue'
import type { GlobalRole, MailAccess, Scope, UserAccount, UserUpdate } from '@/api/contracts'
import { useSandpostClient } from '@/runtime/client'
import { describeError } from '@/utils/errors'
import { cn } from '@/lib/utils'
import { Alert, AlertDescription } from '@/components/ui/alert'
import { Badge } from '@/components/ui/badge'
import { Button } from '@/components/ui/button'
import { Checkbox } from '@/components/ui/checkbox'
import { ancestors, branchContinuations, flattenScopes, siblings, subtreeIds } from './scopeTree'
import { TreeItem, TreeRoot } from 'reka-ui'
import { DialogFooter } from '@/components/ui/dialog'
import { Empty, EmptyDescription, EmptyHeader, EmptyMedia, EmptyTitle } from '@/components/ui/empty'
import { Field, FieldDescription, FieldError, FieldGroup, FieldLabel } from '@/components/ui/field'
import { Input } from '@/components/ui/input'
import { InputGroup, InputGroupAddon, InputGroupInput } from '@/components/ui/input-group'
import { NativeSelect, NativeSelectOption } from '@/components/ui/native-select'
import { Skeleton } from '@/components/ui/skeleton'
import { Spinner } from '@/components/ui/spinner'
import { Table, TableBody, TableCell, TableHead, TableHeader, TableRow } from '@/components/ui/table'
import BusyDialog from '@/components/BusyDialog.vue'
import { useAdminContext } from '@/features/admin/context'

const ROLES: GlobalRole[] = ['owner', 'member', 'viewer']
const MAIL_ACCESS: MailAccess[] = ['all', 'scoped']
const MINIMUM_PASSWORD_BYTES = 12
/** Horizontal indentation added by each scope tree level, in pixels. */
const SCOPE_INDENT = 24
/** Space between a scope row's edge and its checkbox, in pixels. */
const SCOPE_ROW_INSET = 8
/** Distance from a scope row's start to the centre of its checkbox, in pixels. */
const SCOPE_GUIDE_OFFSET = SCOPE_ROW_INSET + 8
/** Width of the elbow joining a parent's guide to a child checkbox, leaving a small gap. */
const SCOPE_ELBOW_WIDTH = SCOPE_INDENT + SCOPE_ROW_INSET - SCOPE_GUIDE_OFFSET - 3

const client = useSandpostClient()
const { t } = useI18n()
const router = useRouter()
const { isGlobalAdmin, user } = useAdminContext()
const users = ref<UserAccount[]>([])
const loading = ref(true)
const error = ref('')

/**
 * The account being edited, or `'new'` while creating one.
 */
const editing = ref<UserAccount | 'new' | null>(null)
const name = ref('')
const email = ref('')
const password = ref('')
const role = ref<GlobalRole>('member')
const mailAccess = ref<MailAccess>('scoped')
const scopes = ref<Scope[]>([])
const selectedScopeIdentifiers = ref<string[]>([])
const savedScopeIdentifiers = ref(new Set<string>())
const scopesLoaded = ref(false)
const loadingScopes = ref(false)
const scopesError = ref('')
const scopeSearch = ref('')
const expandedScopeIdentifiers = ref<string[]>([])
let scopeRequest = 0
const saving = ref(false)
const formError = ref('')
const deleteTarget = ref<UserAccount | null>(null)
const deleting = ref(false)
const deleteError = ref('')

const editingSelf = computed(() => editing.value !== 'new' && editing.value?.id === user.value?.id)
const passwordTooShort = computed(() => password.value.length > 0 && new TextEncoder().encode(password.value).length < MINIMUM_PASSWORD_BYTES)
const grantableRoles = ROLES
const roleDescription = computed(() => t(`admin.users.role.${role.value}Description`))
const mailAccessDescription = computed(() => t(`admin.users.access.${mailAccess.value}Description`))
const formValid = computed(() => name.value.trim() && (editing.value !== 'new' || (email.value.trim() && password.value)) && !passwordTooShort.value && (mailAccess.value !== 'scoped' || scopesLoaded.value))

const filteredScopes = computed(() => {
  const query = scopeSearch.value.trim().toLocaleLowerCase()
  return scopes.value.filter((scope) => scope.name.toLocaleLowerCase().includes(query))
})
const selectedScopes = computed(() => scopes.value.filter((scope) => selectedScopeIdentifiers.value.includes(scope.id)))
const scopeTreeScopes = computed(() => {
  if (!scopeSearch.value.trim()) return scopes.value
  const visible = new Set<string>()
  for (const scope of filteredScopes.value) {
    visible.add(scope.id)
    for (const parent of ancestors(scopes.value, scope.parentId)) visible.add(parent.id)
  }
  return scopes.value.filter((scope) => visible.has(scope.id))
})
const scopeRoots = computed(() => flattenScopes(scopeTreeScopes.value).filter((row) => row.depth === 0).map((row) => row.scope))
const scopeContinuations = computed(() => new Map(scopeTreeScopes.value.map((scope) => [scope.id, branchContinuations(scopeTreeScopes.value, scope)])))

/**
 * Place a tree guide under the checkbox of the row at `level` (1 for roots).
 */
function scopeGuideLeft(level: number): string {
  return `${(level - 1) * SCOPE_INDENT + SCOPE_GUIDE_OFFSET}px`
}

/**
 * Identify a tree item without tying selection to its object identity.
 */
function scopeIdentifier(scope: Scope): string {
  return scope.id
}

/**
 * Return visible children in hierarchy order; leaves have no child collection.
 */
function scopeChildren(scope: Scope): Scope[] | undefined {
  const children = siblings(scopeTreeScopes.value, scope.id)
  return children.length ? children : undefined
}

/**
 * Report whether a collapsed branch hides any selected descendant.
 */
function hasSelectedDescendant(scope: Scope): boolean {
  const descendants = subtreeIds(scopeTreeScopes.value, scope.id)
  descendants.delete(scope.id)
  return selectedScopeIdentifiers.value.some((identifier) => descendants.has(identifier))
}

/**
 * Reflect keyboard or row selection without implicitly assigning any descendants.
 */
function updateScopeSelection(selection: Scope[] | Scope | undefined): void {
  if (!saving.value && scopesLoaded.value) selectedScopeIdentifiers.value = Array.isArray(selection) ? selection.map((scope) => scope.id) : []
}

/**
 * Keep row selection separate from branch expansion; arrows retain native tree keyboard behavior.
 */
function preventScopeClickToggle(event: CustomEvent<{ originalEvent: Event }>): void {
  if (event.detail.originalEvent.type === 'click') event.preventDefault()
}

/**
 * Reveal matching descendants even when their ancestors were previously collapsed.
 */
watch(scopeSearch, () => {
  if (scopeSearch.value.trim()) {
    const parents = filteredScopes.value.flatMap((scope) => ancestors(scopes.value, scope.parentId).map((parent) => parent.id))
    expandedScopeIdentifiers.value = [...new Set([...expandedScopeIdentifiers.value, ...parents])]
  }
})

/**
 * Load current memberships without letting responses from a previously opened form replace its draft.
 */
async function loadUserScopes(account: UserAccount | 'new'): Promise<void> {
  const request = ++scopeRequest
  loadingScopes.value = true
  scopesLoaded.value = false
  scopesError.value = ''
  try {
    const loaded = await client.admin.scopes.list()
    const memberships = account === 'new' ? [] : await Promise.all(loaded.map((scope) => client.admin.scopes.assignments(scope.id)))
    if (request !== scopeRequest || editing.value === null) return
    scopes.value = [...loaded].sort((first, second) => first.name.localeCompare(second.name))
    const assigned = account === 'new' ? [] : loaded.filter((_, index) => memberships[index]?.includes(account.id)).map((scope) => scope.id)
    selectedScopeIdentifiers.value = assigned
    savedScopeIdentifiers.value = new Set(assigned)
    expandedScopeIdentifiers.value = loaded.filter((scope) => loaded.some((child) => child.parentId === scope.id)).map((scope) => scope.id)
    scopesLoaded.value = true
  } catch (failure) {
    if (request === scopeRequest && editing.value !== null) scopesError.value = describeError(failure, t('admin.users.scopesLoadError'))
  } finally {
    if (request === scopeRequest && editing.value !== null) loadingScopes.value = false
  }
}

/**
 * Toggle one draft membership without saving until the account form is submitted.
 */
function toggleScope(identifier: string, checked: boolean | 'indeterminate'): void {
  if (saving.value || !scopesLoaded.value) return
  selectedScopeIdentifiers.value = checked === true
    ? [...new Set([...selectedScopeIdentifiers.value, identifier])]
    : selectedScopeIdentifiers.value.filter((selected) => selected !== identifier)
}

/**
 * Show ancestors to distinguish scopes with the same title without changing inherited access.
 */
function scopeParentPath(scope: Scope): string {
  return ancestors(scopes.value, scope.parentId).map((parent) => parent.name).join(' › ')
}

/**
 * Save only membership differences, retaining each success so a partial failure can be retried.
 */
async function saveScopeAssignments(identifier: string): Promise<void> {
  const desired = new Set(selectedScopeIdentifiers.value)
  for (const scope of [...savedScopeIdentifiers.value]) {
    if (!desired.has(scope)) {
      await client.admin.scopes.unassign(scope, identifier)
      savedScopeIdentifiers.value.delete(scope)
    }
  }
  for (const scope of desired) {
    if (!savedScopeIdentifiers.value.has(scope)) {
      await client.admin.scopes.assign(scope, identifier)
      savedScopeIdentifiers.value.add(scope)
    }
  }
}

/**
 * Load every account, owners first.
 */
async function load(): Promise<void> {
  loading.value = true
  error.value = ''
  try {
    const order: Record<GlobalRole, number> = { owner: 0, member: 1, viewer: 2 }
    users.value = (await client.admin.users.list()).sort((first, second) => order[first.role] - order[second.role] || first.name.localeCompare(second.name))
  } catch (failure) {
    error.value = describeError(failure, t('admin.users.loadError'))
  } finally {
    loading.value = false
  }
}

/**
 * Open the account form for a new or existing account.
 */
function openForm(account: UserAccount | 'new'): void {
  editing.value = account
  name.value = account === 'new' ? '' : account.name
  email.value = account === 'new' ? '' : account.email
  role.value = account === 'new' ? 'member' : account.role
  mailAccess.value = account === 'new' ? 'scoped' : account.mailAccess
  password.value = ''
  formError.value = ''
  scopes.value = []
  selectedScopeIdentifiers.value = []
  savedScopeIdentifiers.value = new Set()
  scopeSearch.value = ''
  void loadUserScopes(account)
}

/**
 * Create the account, or apply the changed fields of an existing one.
 */
async function save(): Promise<void> {
  let target = editing.value
  if (!target || saving.value || !formValid.value) return
  saving.value = true
  formError.value = ''
  let savingAssignments = false
  try {
    let changes: UserUpdate = {}
    if (target === 'new') {
      target = await client.admin.users.create({ name: name.value.trim(), email: email.value.trim(), password: password.value, role: role.value, mailAccess: mailAccess.value })
      // Keep a successfully created account open if assigning its scopes fails, avoiding duplicate creation.
      editing.value = target
      password.value = ''
    } else {
      changes = {
        ...(name.value.trim() !== target.name ? { name: name.value.trim() } : {}),
        ...(role.value !== target.role ? { role: role.value } : {}),
        ...(mailAccess.value !== target.mailAccess ? { mailAccess: mailAccess.value } : {}),
        ...(password.value ? { password: password.value } : {}),
      }
    }
    // All access keeps persisted memberships unchanged for a later switch back to Scoped.
    if (mailAccess.value === 'scoped') {
      savingAssignments = true
      await saveScopeAssignments(target.id)
      savingAssignments = false
    }
    // Memberships must be saved before an account update can revoke the current Owner's session.
    const accountUpdated = Object.keys(changes).length > 0
    if (accountUpdated) await client.admin.users.update(target.id, changes)
    const signedOut = accountUpdated && editingSelf.value
    editing.value = null
    if (signedOut) { await router.replace({ name: 'login' }); return }
    await load()
  } catch (failure) {
    const message = describeError(failure, t('admin.users.saveError'))
    formError.value = savingAssignments
      ? `${t('admin.users.scopesSaveError')} ${message}`
      : message === 'Conflict' ? t('admin.users.conflict') : message
  } finally {
    saving.value = false
  }
}

/**
 * Delete the confirmed account.
 */
async function remove(): Promise<void> {
  const target = deleteTarget.value
  if (!target || deleting.value) return
  deleting.value = true
  deleteError.value = ''
  try {
    await client.admin.users.delete(target.id)
    deleteTarget.value = null
    await load()
  } catch (failure) {
    deleteError.value = describeError(failure, t('admin.users.deleteError'))
  } finally {
    deleting.value = false
  }
}

onBeforeUnmount(() => { scopeRequest += 1 })
onMounted(load)
</script>

<template>
  <div class="mx-auto flex max-w-6xl flex-col gap-6 p-4 sm:p-6">
    <div class="flex flex-wrap items-end justify-between gap-3">
      <div class="flex flex-col gap-1">
    <h1 class="text-lg font-semibold tracking-tight">{{ t('admin.users.title') }}</h1>
        <p class="text-sm text-muted-foreground">{{ t('admin.users.description') }}</p>
      </div>
      <Button v-if="isGlobalAdmin" @click="openForm('new')"><PlusIcon data-icon="inline-start" />{{ t('admin.users.new') }}</Button>
    </div>

    <Empty v-if="!isGlobalAdmin" class="border">
      <EmptyHeader>
        <EmptyMedia variant="icon"><UsersIcon /></EmptyMedia>
        <EmptyTitle>{{ t('admin.users.onlyAdmins') }}</EmptyTitle>
        <EmptyDescription>{{ t('admin.users.adminsManage') }}</EmptyDescription>
      </EmptyHeader>
    </Empty>
    <template v-else>
      <Alert v-if="error" variant="destructive" role="alert">
        <AlertDescription class="flex items-center gap-3"><span class="flex-1">{{ error }}</span><Button variant="outline" size="sm" @click="load">{{ t('admin.users.retry') }}</Button></AlertDescription>
      </Alert>
      <div v-if="loading" class="flex flex-col gap-2"><Skeleton v-for="index in 4" :key="index" class="h-12 w-full" /></div>
      <div v-else class="overflow-hidden rounded-lg border">
        <Table>
          <TableHeader>
            <TableRow>
              <TableHead>{{ t('admin.users.name') }}</TableHead>
              <TableHead class="max-sm:hidden">{{ t('admin.users.email') }}</TableHead>
              <TableHead class="w-28">{{ t('admin.users.roleLabel') }}</TableHead>
              <TableHead class="w-28 max-sm:hidden">{{ t('admin.users.mailAccess') }}</TableHead>
              <TableHead class="w-24"><span class="sr-only">{{ t('admin.users.actions') }}</span></TableHead>
            </TableRow>
          </TableHeader>
          <TableBody>
            <TableRow v-for="account in users" :key="account.id">
              <TableCell class="max-w-48">
                <div class="flex min-w-0 flex-col">
                  <span class="truncate font-medium">{{ account.name }}<span v-if="account.id === user?.id" class="font-normal text-muted-foreground">{{ ' ' + t('admin.users.you') }}</span></span>
                  <span class="truncate text-xs text-muted-foreground sm:hidden">{{ account.email }}</span>
                </div>
              </TableCell>
              <TableCell class="text-muted-foreground max-sm:hidden">{{ account.email }}</TableCell>
              <TableCell><Badge :variant="account.role === 'member' || account.role === 'viewer' ? 'outline' : 'secondary'">{{ t(`admin.users.role.${account.role}`) }}</Badge></TableCell>
              <TableCell class="text-muted-foreground max-sm:hidden">{{ t(`admin.users.${account.mailAccess === 'all' ? 'allMail' : 'scoped'}`) }}</TableCell>
              <TableCell>
                <div class="flex justify-end gap-1">
                  <Button variant="ghost" size="icon-sm" :aria-label="t('admin.users.editName', { name: account.name })" @click="openForm(account)"><PencilIcon /></Button>
                  <Button variant="ghost" size="icon-sm" :aria-label="t('admin.users.deleteName', { name: account.name })" :disabled="account.id === user?.id" @click="deleteError = ''; deleteTarget = account"><Trash2Icon /></Button>
                </div>
              </TableCell>
            </TableRow>
          </TableBody>
        </Table>
      </div>
    </template>

    <BusyDialog
      :open="Boolean(editing)"
      :title="editing === 'new' ? t('admin.users.new') : t('admin.users.editTitle')"
      :description="editing === 'new' ? t('admin.users.newDescription') : t('admin.users.editDescription')"
      :busy="saving"
      @update:open="!$event && (editing = null)"
    >
      <form class="flex flex-col gap-4" @submit.prevent="save">
        <FieldGroup class="gap-4">
          <Field :data-disabled="saving">
          <FieldLabel for="user-name">{{ t('admin.users.name') }}</FieldLabel>
            <Input id="user-name" v-model="name" required maxlength="100" autocomplete="off" :disabled="saving" />
          </Field>
          <Field :data-disabled="saving || editing !== 'new'">
          <FieldLabel for="user-email">{{ t('admin.users.email') }}</FieldLabel>
            <Input id="user-email" v-model="email" type="email" required autocomplete="off" :disabled="saving || editing !== 'new'" />
            <FieldDescription v-if="editing !== 'new'">{{ t('admin.users.emailImmutable') }}</FieldDescription>
          </Field>
          <Field :data-disabled="saving" :data-invalid="passwordTooShort">
            <FieldLabel for="user-password">{{ t(editing === 'new' ? 'admin.users.password' : 'admin.users.newPassword') }}</FieldLabel>
            <Input id="user-password" v-model="password" type="password" autocomplete="new-password" :required="editing === 'new'" :placeholder="editing === 'new' ? '' : t('admin.users.leavePassword')" :disabled="saving" :aria-invalid="passwordTooShort" />
            <FieldError v-if="passwordTooShort">{{ t('admin.users.minPassword', { count: MINIMUM_PASSWORD_BYTES }) }}</FieldError>
          </Field>
          <Field :data-disabled="saving">
            <FieldLabel for="user-role">{{ t('admin.users.roleLabel') }}</FieldLabel>
            <NativeSelect id="user-role" v-model="role" class="w-full" :disabled="saving">
              <NativeSelectOption v-for="option in grantableRoles" :key="option" :value="option">{{ t(`admin.users.role.${option}`) }}</NativeSelectOption>
            </NativeSelect>
            <FieldDescription>{{ roleDescription }}</FieldDescription>
          </Field>
          <Field :data-disabled="saving">
            <FieldLabel for="user-mail-access">{{ t('admin.users.mailAccess') }}</FieldLabel>
            <NativeSelect id="user-mail-access" v-model="mailAccess" class="w-full" :disabled="saving">
              <NativeSelectOption v-for="option in MAIL_ACCESS" :key="option" :value="option">{{ t(`admin.users.${option === 'all' ? 'allMail' : 'scoped'}`) }}</NativeSelectOption>
            </NativeSelect>
            <FieldDescription>{{ mailAccessDescription }}</FieldDescription>
          </Field>
          <Field v-if="mailAccess === 'scoped'" :data-disabled="saving || loadingScopes">
            <div class="flex items-baseline justify-between gap-2">
              <FieldLabel for="user-scopes">{{ t('admin.users.scopes') }}</FieldLabel>
              <span v-if="scopesLoaded && selectedScopes.length" class="text-xs text-muted-foreground" aria-live="polite">{{ t('admin.users.selectedScopes', { count: selectedScopes.length }) }}</span>
            </div>
            <p v-if="loadingScopes" class="flex items-center gap-2 text-sm text-muted-foreground" role="status"><Spinner aria-hidden="true" />{{ t('admin.users.loadingScopes') }}</p>
            <FieldError v-else-if="scopesError" role="alert">
              {{ scopesError }}
              <Button type="button" variant="outline" size="sm" :disabled="saving" @click="editing && loadUserScopes(editing)">{{ t('admin.users.retry') }}</Button>
            </FieldError>
            <template v-else-if="scopesLoaded">
              <InputGroup>
                <InputGroupAddon><SearchIcon aria-hidden="true" /></InputGroupAddon>
                <InputGroupInput id="user-scopes" v-model="scopeSearch" type="search" :placeholder="t('admin.users.searchScopes')" :disabled="saving" aria-describedby="user-scopes-hint" />
              </InputGroup>
              <div class="rounded-md border bg-background/40">
                <TreeRoot
                  v-slot="{ flattenItems }"
                  v-model:expanded="expandedScopeIdentifiers"
                  :items="scopeRoots"
                  :model-value="selectedScopes"
                  :get-key="scopeIdentifier"
                  :get-children="scopeChildren"
                  :disabled="saving"
                  multiple
                  selection-behavior="toggle"
                  class="flex max-h-72 flex-col overflow-y-auto p-1"
                  :aria-label="t('admin.users.scopes')"
                  @update:model-value="updateScopeSelection"
                >
                  <TransitionGroup
                    enter-active-class="overflow-hidden"
                    enter-from-class="h-0! opacity-0"
                    leave-active-class="overflow-hidden"
                    leave-to-class="h-0! opacity-0"
                  >
                    <TreeItem
                      v-for="item in flattenItems"
                      :key="item._id"
                      v-slot="{ isExpanded, isSelected, handleToggle }"
                      v-bind="item.bind"
                      :aria-label="item.value.name"
                      class="group/scope relative flex h-8 shrink-0 cursor-pointer items-center gap-2 rounded-md pr-1 text-sm text-muted-foreground outline-none transition-[color,height,opacity] duration-200 ease-out motion-reduce:transition-none hover:text-foreground focus-visible:ring-2 focus-visible:ring-ring data-[selected]:text-foreground"
                      :style="{ paddingInlineStart: `${(item.level - 1) * SCOPE_INDENT + SCOPE_ROW_INSET}px` }"
                      @toggle="preventScopeClickToggle"
                    >
                      <template v-if="item.level > 1">
                        <template v-for="(continues, column) in scopeContinuations.get(item.value.id)?.slice(1, -1)" :key="column">
                          <span v-if="continues" class="pointer-events-none absolute inset-y-0 w-px bg-muted-foreground/30" :style="{ left: scopeGuideLeft(column + 1) }" aria-hidden="true" />
                        </template>
                        <span class="pointer-events-none absolute top-0 h-1/2 rounded-bl-[5px] border-b border-l border-muted-foreground/30" :style="{ left: scopeGuideLeft(item.level - 1), width: `${SCOPE_ELBOW_WIDTH}px` }" aria-hidden="true" />
                        <span v-if="scopeContinuations.get(item.value.id)?.at(-1)" class="pointer-events-none absolute top-1/2 bottom-0 w-px bg-muted-foreground/30" :style="{ left: scopeGuideLeft(item.level - 1) }" aria-hidden="true" />
                      </template>
                      <span v-if="item.hasChildren" :class="cn('pointer-events-none absolute bottom-0 top-[calc(50%+11px)] w-px bg-muted-foreground/30 transition-opacity duration-150', !isExpanded && 'opacity-0')" :style="{ left: scopeGuideLeft(item.level) }" aria-hidden="true" />
                      <Checkbox :id="`user-scope-${item.value.id}`" :model-value="isSelected" :disabled="saving" :tabindex="-1" aria-hidden="true" class="pointer-events-none" @update:model-value="toggleScope(item.value.id, $event)" />
                      <span :class="cn('min-w-0 flex-1 truncate', item.hasChildren && 'font-medium')" :title="scopeParentPath(item.value)">{{ item.value.name }}</span>
                      <span v-if="item.hasChildren && !isExpanded" class="flex items-center gap-1.5 text-xs tabular-nums text-muted-foreground" aria-hidden="true">
                        <span v-if="hasSelectedDescendant(item.value)" class="size-1.5 rounded-full bg-primary" data-selected-descendant />
                        {{ scopeChildren(item.value)?.length }}
                      </span>
                      <Button v-if="item.hasChildren" type="button" variant="ghost" size="icon-sm" class="size-6 shrink-0 text-muted-foreground hover:text-foreground" :tabindex="-1" :disabled="saving" :aria-label="t(isExpanded ? 'admin.users.collapseScope' : 'admin.users.expandScope', { name: item.value.name })" @click.stop="handleToggle">
                        <ChevronRightIcon :class="cn('transition-transform duration-150', isExpanded && 'rotate-90')" aria-hidden="true" />
                      </Button>
                    </TreeItem>
                  </TransitionGroup>
                </TreeRoot>
                <p v-if="!filteredScopes.length" class="px-3 py-4 text-center text-sm text-muted-foreground" role="status">{{ t(scopes.length ? 'admin.users.noMatchingScopes' : 'admin.users.noScopes') }}</p>
              </div>
            </template>
            <FieldDescription id="user-scopes-hint">{{ t('admin.users.scopesHint') }}</FieldDescription>
          </Field>
          <Alert v-if="editingSelf"><AlertDescription>{{ t('admin.users.selfEdit') }}</AlertDescription></Alert>
          <FieldError v-if="formError" role="alert">{{ formError }}</FieldError>
        </FieldGroup>
        <DialogFooter>
          <Button variant="outline" type="button" :disabled="saving" @click="editing = null">{{ t('admin.users.cancel') }}</Button>
          <Button type="submit" :disabled="saving || !formValid">
            <Spinner v-if="saving" data-icon="inline-start" aria-hidden="true" />{{ saving ? t('admin.users.saving') : editing === 'new' ? t('admin.users.new') : t('admin.users.save') }}
          </Button>
        </DialogFooter>
      </form>
    </BusyDialog>

    <BusyDialog :open="Boolean(deleteTarget)" :title="t('admin.users.deleteTitle', { name: deleteTarget?.name ?? t('admin.users.title') })" :description="t('admin.users.deleteDescription')" :busy="deleting" @update:open="!$event && (deleteTarget = null)">
      <Alert v-if="deleteError" variant="destructive" role="alert"><AlertDescription>{{ deleteError }}</AlertDescription></Alert>
      <DialogFooter>
          <Button variant="outline" :disabled="deleting" @click="deleteTarget = null">{{ t('admin.users.cancel') }}</Button>
        <Button variant="destructive" :disabled="deleting" @click="remove">
          <Spinner v-if="deleting" data-icon="inline-start" aria-hidden="true" />{{ deleting ? t('admin.users.deleting') : t('admin.users.deleteUser') }}
        </Button>
      </DialogFooter>
    </BusyDialog>
  </div>
</template>
