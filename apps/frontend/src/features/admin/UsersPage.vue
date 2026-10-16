<script setup lang="ts">
import { computed, onMounted, ref } from 'vue'
import { useI18n } from 'vue-i18n'
import { useRouter } from 'vue-router'
import { PencilIcon, PlusIcon, Trash2Icon, UsersIcon } from '@lucide/vue'
import type { GlobalRole, MailAccess, UserAccount } from '@/api/contracts'
import { useSandpostClient } from '@/runtime/client'
import { describeError } from '@/utils/errors'
import { Alert, AlertDescription } from '@/components/ui/alert'
import { Badge } from '@/components/ui/badge'
import { Button } from '@/components/ui/button'
import { DialogFooter } from '@/components/ui/dialog'
import { Empty, EmptyDescription, EmptyHeader, EmptyMedia, EmptyTitle } from '@/components/ui/empty'
import { Field, FieldDescription, FieldError, FieldGroup, FieldLabel } from '@/components/ui/field'
import { Input } from '@/components/ui/input'
import { NativeSelect, NativeSelectOption } from '@/components/ui/native-select'
import { Skeleton } from '@/components/ui/skeleton'
import { Spinner } from '@/components/ui/spinner'
import { Table, TableBody, TableCell, TableHead, TableHeader, TableRow } from '@/components/ui/table'
import BusyDialog from '@/components/BusyDialog.vue'
import { useAdminContext } from '@/features/admin/context'

const ROLES: GlobalRole[] = ['owner', 'admin', 'member', 'viewer']
const MAIL_ACCESS: MailAccess[] = ['all', 'scoped']
const MINIMUM_PASSWORD_BYTES = 12

const client = useSandpostClient()
const { t } = useI18n()
const router = useRouter()
const { isGlobalAdmin, isOwner, user } = useAdminContext()
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
const saving = ref(false)
const formError = ref('')
const deleteTarget = ref<UserAccount | null>(null)
const deleting = ref(false)
const deleteError = ref('')

const editingSelf = computed(() => editing.value !== 'new' && editing.value?.id === user.value?.id)
const passwordTooShort = computed(() => password.value.length > 0 && new TextEncoder().encode(password.value).length < MINIMUM_PASSWORD_BYTES)
const grantableRoles = computed(() => ROLES.filter((option) => isOwner.value || option !== 'owner'))
const roleDescription = computed(() => t(`admin.users.role.${role.value}Description`))
const mailAccessDescription = computed(() => role.value === 'owner'
  ? t('admin.users.access.ownerDescription')
  : t(`admin.users.access.${mailAccess.value}Description`))
const formValid = computed(() => name.value.trim() && (editing.value !== 'new' || (email.value.trim() && password.value)) && !passwordTooShort.value)

/**
 * Report whether the current administrator may change this account.
 */
function manageable(account: UserAccount): boolean {
  return isOwner.value || account.role !== 'owner'
}

/**
 * Load every account, owners first.
 */
async function load(): Promise<void> {
  loading.value = true
  error.value = ''
  try {
    const order: Record<GlobalRole, number> = { owner: 0, admin: 1, member: 2, viewer: 3 }
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
}

/**
 * Create the account, or apply the changed fields of an existing one.
 */
async function save(): Promise<void> {
  const target = editing.value
  if (!target || saving.value || !formValid.value) return
  saving.value = true
  formError.value = ''
  try {
    if (target === 'new') {
      await client.admin.users.create({ name: name.value.trim(), email: email.value.trim(), password: password.value, role: role.value, mailAccess: mailAccess.value })
    } else {
      await client.admin.users.update(target.id, {
        ...(name.value.trim() !== target.name ? { name: name.value.trim() } : {}),
        ...(role.value !== target.role ? { role: role.value } : {}),
        ...(mailAccess.value !== target.mailAccess ? { mailAccess: mailAccess.value } : {}),
        ...(password.value ? { password: password.value } : {}),
      })
    }
    const signedOut = editingSelf.value
    editing.value = null
    /*
     * Account changes revoke the edited user's sessions, including the current one.
     */
    if (signedOut) { await router.replace({ name: 'login' }); return }
    await load()
  } catch (failure) {
    const message = describeError(failure, t('admin.users.saveError'))
    formError.value = message === 'Conflict' ? t('admin.users.conflict') : message
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
              <TableCell class="text-muted-foreground max-sm:hidden">{{ account.role === 'owner' || account.mailAccess === 'all' ? t('admin.users.allMail') : t('admin.users.scoped') }}</TableCell>
              <TableCell>
                <div class="flex justify-end gap-1">
                  <Button variant="ghost" size="icon-sm" :aria-label="t('admin.users.editName', { name: account.name })" :disabled="!manageable(account)" @click="openForm(account)"><PencilIcon /></Button>
                  <Button variant="ghost" size="icon-sm" :aria-label="t('admin.users.deleteName', { name: account.name })" :disabled="!manageable(account) || account.id === user?.id" @click="deleteError = ''; deleteTarget = account"><Trash2Icon /></Button>
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
          <Field :data-disabled="saving || role === 'owner'">
            <FieldLabel for="user-mail-access">{{ t('admin.users.mailAccess') }}</FieldLabel>
            <NativeSelect id="user-mail-access" v-model="mailAccess" class="w-full" :disabled="saving || role === 'owner'">
              <NativeSelectOption v-for="option in MAIL_ACCESS" :key="option" :value="option">{{ t(`admin.users.${option === 'all' ? 'allMail' : 'scoped'}`) }}</NativeSelectOption>
            </NativeSelect>
            <FieldDescription>{{ mailAccessDescription }}</FieldDescription>
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
