<script setup lang="ts">
import { computed, onMounted, ref } from 'vue'
import { useI18n } from 'vue-i18n'
import {
  CheckIcon, CircleCheckIcon, CircleXIcon, CopyIcon, KeyRoundIcon, LockIcon, MoreHorizontalIcon, PlusIcon, PowerIcon, PowerOffIcon,
  RefreshCwIcon, ShieldCheckIcon, Trash2Icon,
} from '@lucide/vue'
import type { IssuedSmtpCredentials, SmtpAccess, SmtpMechanism, SmtpServer } from '@/api/contracts'
import { SMTP_MECHANISMS } from '@/api/admin'
import { useSandpostClient } from '@/runtime/client'
import { asSentence, describeError } from '@/utils/errors'
import { formatFullDate, formatListDate } from '@/utils/format'
import { Alert, AlertDescription, AlertTitle } from '@/components/ui/alert'
import { Badge } from '@/components/ui/badge'
import { Button } from '@/components/ui/button'
import { Card, CardAction, CardContent, CardDescription, CardHeader, CardTitle } from '@/components/ui/card'
import { DialogFooter } from '@/components/ui/dialog'
import {
  DropdownMenu, DropdownMenuContent, DropdownMenuGroup, DropdownMenuItem, DropdownMenuSeparator, DropdownMenuTrigger,
} from '@/components/ui/dropdown-menu'
import { Empty, EmptyContent, EmptyDescription, EmptyHeader, EmptyMedia, EmptyTitle } from '@/components/ui/empty'
import { Field, FieldDescription, FieldError, FieldGroup, FieldLabel } from '@/components/ui/field'
import { Input } from '@/components/ui/input'
import { Skeleton } from '@/components/ui/skeleton'
import { Spinner } from '@/components/ui/spinner'
import { Table, TableBody, TableCell, TableHead, TableHeader, TableRow } from '@/components/ui/table'
import BusyDialog from '@/components/BusyDialog.vue'
import ListenerBadge from '@/features/admin/ListenerBadge.vue'
import SmtpAccessRulesFields from '@/features/admin/SmtpAccessRulesFields.vue'
import SmtpCredentialsDialog from '@/features/admin/SmtpCredentialsDialog.vue'
import { useAdminContext } from '@/features/admin/context'
import { connectionProtection, credentialProtection } from '@/features/admin/encryption'
import { configuredListenerAddress, connectionAddress } from '@/features/admin/listenAddress'
import { useCopy } from '@/composables/useCopy'

const client = useSandpostClient()
const { t } = useI18n()
const { isGlobalAdmin } = useAdminContext()
const accesses = ref<SmtpAccess[]>([])
const server = ref<SmtpServer | null>(null)
const loading = ref(true)
const error = ref('')
const rowError = ref('')

const createOpen = ref(false)
const creating = ref(false)
const createError = ref('')
const name = ref('')
const username = ref('')
const createRequiresEncryption = ref(false)
const createMechanisms = ref<SmtpMechanism[]>([...SMTP_MECHANISMS])

/**
 * The access whose authentication rules are being edited.
 */
const rulesTarget = ref<SmtpAccess | null>(null)
const rulesRequiresEncryption = ref(false)
const rulesMechanisms = ref<SmtpMechanism[]>([])
const savingRules = ref(false)
const rulesError = ref('')

/**
 * Credentials shown once by the dialog; cleared as soon as it closes.
 */
const issued = ref<IssuedSmtpCredentials | null>(null)
const regenerated = ref(false)
const regenerateTarget = ref<SmtpAccess | null>(null)
const regenerating = ref(false)
const regenerateError = ref('')
const deleteTarget = ref<SmtpAccess | null>(null)
const deleting = ref(false)
const deleteError = ref('')
const toggling = ref<string | null>(null)
const { copied: copiedUsername, copy } = useCopy()

const listener = computed(() => server.value?.listener ?? null)
const configuredBindAddress = computed(() => server.value ? configuredListenerAddress(server.value) : null)
const connection = computed(() => {
  const address = listener.value?.boundAddress ?? listener.value?.configuredAddress ?? server.value?.listenAddress
  return address ? connectionAddress(address) : null
})

/**
 * Whether the listener can encrypt sessions, which TLS-only accesses depend on.
 */
const encryptionAvailable = computed(() => (listener.value?.encryption ?? 'none') !== 'none')
const rulesChanged = computed(() => Boolean(rulesTarget.value) && (
  rulesRequiresEncryption.value !== rulesTarget.value?.requiresEncryption
  || rulesMechanisms.value.join() !== rulesTarget.value?.allowedMechanisms.join()
))

/**
 * Summarize how an access may authenticate for the table.
 */
function rulesSummary(access: SmtpAccess): string {
  const mechanisms = access.allowedMechanisms.map((mechanism) => mechanism.toUpperCase()).join(' · ')
  return access.requiresEncryption ? `${mechanisms} · ${t('admin.smtp.tlsOnly')}` : mechanisms
}

/**
 * Load the global server and its accesses.
 */
async function load(): Promise<void> {
  loading.value = true
  error.value = ''
  try {
    const [loadedServer, loadedAccesses] = await Promise.all([client.admin.smtp.server(), client.admin.smtp.list()])
    server.value = loadedServer
    accesses.value = loadedAccesses
  } catch (failure) {
    error.value = describeError(failure, t('admin.smtp.loadError'))
  } finally {
    loading.value = false
  }
}

/**
 * Open an empty creation dialog.
 */
function openCreate(): void {
  name.value = ''
  username.value = ''
  /*
   * New accesses are TLS-only whenever the server can encrypt.
   */
  createRequiresEncryption.value = encryptionAvailable.value
  createMechanisms.value = [...SMTP_MECHANISMS]
  createError.value = ''
  createOpen.value = true
}

/**
 * Create the access and show its generated password once.
 */
async function create(): Promise<void> {
  if (creating.value || !name.value.trim() || !createMechanisms.value.length) return
  creating.value = true
  createError.value = ''
  try {
    const credentials = await client.admin.smtp.create({
      name: name.value.trim(),
      username: username.value.trim() || null,
      requiresEncryption: createRequiresEncryption.value,
      allowedMechanisms: createMechanisms.value,
    })
    createOpen.value = false
    regenerated.value = false
    issued.value = credentials
    await load()
  } catch (failure) {
    const message = describeError(failure, t('admin.smtp.createError'))
    createError.value = message === 'Conflict' ? t('admin.smtp.createConflict') : message
  } finally {
    creating.value = false
  }
}

/**
 * Replace the confirmed access's password and show the new one once.
 */
async function regenerate(): Promise<void> {
  const target = regenerateTarget.value
  if (!target || regenerating.value) return
  regenerating.value = true
  regenerateError.value = ''
  try {
    const credentials = await client.admin.smtp.regeneratePassword(target.id)
    regenerateTarget.value = null
    regenerated.value = true
    issued.value = credentials
    await load()
  } catch (failure) {
    regenerateError.value = describeError(failure, t('admin.smtp.regenerateError'))
  } finally {
    regenerating.value = false
  }
}

/**
 * Enable or disable an access in place.
 */
async function toggle(access: SmtpAccess): Promise<void> {
  if (toggling.value) return
  toggling.value = access.id
  rowError.value = ''
  try {
    const updated = await client.admin.smtp.setEnabled(access.id, !access.enabled)
    accesses.value = accesses.value.map((candidate) => candidate.id === updated.id ? updated : candidate)
  } catch (failure) {
    rowError.value = describeError(failure, t('admin.smtp.updateError'))
  } finally {
    toggling.value = null
  }
}

/**
 * Open the authentication settings of an access.
 */
function openRules(access: SmtpAccess): void {
  rulesTarget.value = access
  rulesRequiresEncryption.value = access.requiresEncryption
  rulesMechanisms.value = [...access.allowedMechanisms]
  rulesError.value = ''
}

/**
 * Save the edited authentication rules; non-compliant sessions stop delivering at once.
 */
async function saveRules(): Promise<void> {
  const target = rulesTarget.value
  if (!target || savingRules.value || !rulesMechanisms.value.length) return
  savingRules.value = true
  rulesError.value = ''
  try {
    const updated = await client.admin.smtp.updateRules(target.id, {
      requiresEncryption: rulesRequiresEncryption.value,
      allowedMechanisms: rulesMechanisms.value,
    })
    accesses.value = accesses.value.map((candidate) => candidate.id === updated.id ? updated : candidate)
    rulesTarget.value = null
  } catch (failure) {
    rulesError.value = describeError(failure, t('admin.smtp.rulesError'))
  } finally {
    savingRules.value = false
  }
}

/**
 * Delete the confirmed access.
 */
async function remove(): Promise<void> {
  const target = deleteTarget.value
  if (!target || deleting.value) return
  deleting.value = true
  deleteError.value = ''
  try {
    await client.admin.smtp.delete(target.id)
    deleteTarget.value = null
    await load()
  } catch (failure) {
    deleteError.value = describeError(failure, t('admin.smtp.deleteError'))
  } finally {
    deleting.value = false
  }
}

/**
 * Copy a username, briefly confirming it in the open menu or reporting clipboard failures inline.
 */
async function copyUsername(access: SmtpAccess): Promise<void> {
  rowError.value = ''
  if (!await copy(access.username, access.id)) rowError.value = 'Could not copy the username. Check clipboard permissions and try again.'
}

/**
 * Forget the issued password as soon as its dialog closes.
 */
function credentialsClosed(): void {
  issued.value = null
  regenerated.value = false
}

onMounted(() => { if (isGlobalAdmin.value) void load() })
</script>

<template>
  <div class="mx-auto flex max-w-6xl flex-col gap-6 p-4 sm:p-6">
    <div class="flex flex-wrap items-end justify-between gap-3">
      <div class="flex flex-col gap-1">
    <h1 class="text-lg font-semibold tracking-tight">{{ t('admin.smtp.title') }}</h1>
        <p class="text-sm text-muted-foreground">{{ t('admin.smtp.description') }}</p>
      </div>
      <Button v-if="isGlobalAdmin" :disabled="loading || Boolean(error)" @click="openCreate"><PlusIcon data-icon="inline-start" />{{ t('admin.smtp.createAccess') }}</Button>
    </div>

    <Empty v-if="!isGlobalAdmin" class="border">
      <EmptyHeader>
        <EmptyMedia variant="icon"><KeyRoundIcon /></EmptyMedia>
        <EmptyTitle>{{ t('admin.smtp.onlyAdmins') }}</EmptyTitle>
        <EmptyDescription>{{ t('admin.smtp.adminsManage') }}</EmptyDescription>
      </EmptyHeader>
    </Empty>
    <template v-else>
      <Alert v-if="error" variant="destructive" role="alert">
        <AlertDescription class="flex items-center gap-3"><span class="flex-1">{{ error }}</span><Button variant="outline" size="sm" @click="load">{{ t('admin.smtp.retry') }}</Button></AlertDescription>
      </Alert>

      <div v-if="loading" class="flex flex-col gap-3">
        <Skeleton class="h-40 w-full" />
        <Skeleton v-for="index in 3" :key="index" class="h-12 w-full" />
      </div>
      <template v-else-if="server">
        <Card>
          <CardHeader>
            <CardTitle>{{ t('admin.smtp.server') }}</CardTitle>
            <CardDescription>
              <template v-if="connection">{{ t('admin.smtp.applicationConnect') }} <span class="font-mono text-foreground">{{ connection.host }}:{{ connection.port }}</span> {{ connectionProtection(listener?.encryption ?? 'none') }}</template>
              <template v-else>{{ t('admin.smtp.disabled') }}</template>
            </CardDescription>
            <CardAction><ListenerBadge :server="server" /></CardAction>
          </CardHeader>
          <CardContent class="flex flex-col gap-4">
            <dl class="flex flex-col gap-1 rounded-md border px-3 py-2">
              <dt class="text-xs text-muted-foreground">{{ t('admin.smtp.configuredBindAddress') }}</dt>
              <dd class="break-all font-mono text-sm">{{ configuredBindAddress ?? '—' }}</dd>
            </dl>
            <Alert v-if="listener?.state === 'failed'" variant="destructive">
              <CircleXIcon />
              <AlertTitle>{{ t('admin.smtp.notListening') }}</AlertTitle>
              <AlertDescription>
                <span class="block">{{ asSentence(listener.error ?? t('admin.smtp.listenerStartError')) }}</span>
              </AlertDescription>
            </Alert>
            <Alert v-else-if="listener?.state === 'listening'">
              <component :is="listener.authenticationRequired ? LockIcon : CircleCheckIcon" />
              <AlertTitle>{{ listener.authenticationRequired ? t('admin.smtp.authenticationRequired') : t('admin.smtp.localMail') }}</AlertTitle>
              <AlertDescription>
                {{ listener.authenticationRequired
                  ? t('admin.smtp.authRequired', { protection: credentialProtection(listener.encryption) })
                  : t('admin.smtp.noTls') }}
              </AlertDescription>
            </Alert>
            <Alert v-if="listener?.source === 'configuration'">
              <LockIcon />
              <AlertTitle>{{ t('admin.smtp.configuration') }}</AlertTitle>
              <AlertDescription>
                {{ t('admin.smtp.configurationDescription') }}
              </AlertDescription>
            </Alert>
          </CardContent>
        </Card>

        <Alert v-if="rowError" variant="destructive" role="alert"><AlertDescription>{{ rowError }}</AlertDescription></Alert>
        <Empty v-if="!accesses.length" class="border">
          <EmptyHeader>
            <EmptyMedia variant="icon"><KeyRoundIcon /></EmptyMedia>
            <EmptyTitle>{{ t('admin.smtp.noAccesses') }}</EmptyTitle>
            <EmptyDescription>{{ t('admin.smtp.noAccessDescription') }}</EmptyDescription>
          </EmptyHeader>
          <EmptyContent><Button @click="openCreate"><PlusIcon data-icon="inline-start" />{{ t('admin.smtp.createAccess') }}</Button></EmptyContent>
        </Empty>
        <div v-else class="overflow-hidden rounded-lg border">
          <Table>
            <TableHeader>
              <TableRow>
                <TableHead>{{ t('admin.smtp.name') }}</TableHead>
                <TableHead>{{ t('admin.smtp.username') }}</TableHead>
                <TableHead class="max-lg:hidden">{{ t('admin.smtp.authentication') }}</TableHead>
                <TableHead class="w-24">{{ t('admin.smtp.status') }}</TableHead>
                <TableHead class="max-md:hidden">{{ t('admin.smtp.created') }}</TableHead>
                <TableHead class="max-md:hidden">{{ t('admin.smtp.lastUsed') }}</TableHead>
                <TableHead class="w-12"><span class="sr-only">{{ t('admin.users.actions') }}</span></TableHead>
              </TableRow>
            </TableHeader>
            <TableBody>
              <TableRow v-for="access in accesses" :key="access.id">
                <TableCell class="max-w-48">
                  <span class="block truncate font-medium">{{ access.name }}</span>
                </TableCell>
                <TableCell class="max-w-56"><span class="block truncate font-mono text-xs">{{ access.username }}</span></TableCell>
                <TableCell class="max-lg:hidden">
                  <span class="inline-flex items-center gap-1.5 text-xs text-muted-foreground">
                    <ShieldCheckIcon v-if="access.requiresEncryption" class="size-3.5 text-foreground" aria-hidden="true" />{{ rulesSummary(access) }}
                  </span>
                </TableCell>
                <TableCell>
                  <Badge :variant="access.enabled ? 'secondary' : 'outline'">{{ access.enabled ? t('admin.smtp.active') : t('admin.smtp.disabledStatus') }}</Badge>
                </TableCell>
                <TableCell class="text-muted-foreground max-md:hidden" :title="formatFullDate(access.createdAt)">{{ formatListDate(access.createdAt) }}</TableCell>
                <TableCell class="text-muted-foreground max-md:hidden" :title="access.lastUsedAt ? formatFullDate(access.lastUsedAt) : undefined">
                  {{ access.lastUsedAt ? formatListDate(access.lastUsedAt) : t('admin.smtp.never') }}
                </TableCell>
                <TableCell>
                  <div class="flex justify-end">
                    <DropdownMenu>
                      <DropdownMenuTrigger as-child>
                        <Button variant="ghost" size="icon-sm" :aria-label="t('admin.smtp.nameActions', { name: access.name })" :disabled="toggling === access.id">
                          <Spinner v-if="toggling === access.id" />
                          <MoreHorizontalIcon v-else />
                        </Button>
                      </DropdownMenuTrigger>
                      <DropdownMenuContent align="end" class="w-48">
                        <DropdownMenuGroup>
                          <DropdownMenuItem @select="(event: Event) => { event.preventDefault(); void copyUsername(access) }">
                            <CheckIcon v-if="copiedUsername === access.id" class="text-success! *:text-success!" /><CopyIcon v-else />{{ copiedUsername === access.id ? t('admin.smtp.copied') : t('admin.smtp.copyUsername') }}
                          </DropdownMenuItem>
                          <DropdownMenuItem @select="regenerateError = ''; regenerateTarget = access"><RefreshCwIcon />{{ t('admin.smtp.regenerate') }}</DropdownMenuItem>
                          <DropdownMenuItem @select="openRules(access)"><ShieldCheckIcon />{{ t('admin.smtp.settings') }}</DropdownMenuItem>
                          <DropdownMenuItem @select="toggle(access)">
                            <component :is="access.enabled ? PowerOffIcon : PowerIcon" />{{ access.enabled ? t('admin.smtp.disable') : t('admin.smtp.enable') }}
                          </DropdownMenuItem>
                        </DropdownMenuGroup>
                        <DropdownMenuSeparator />
                        <DropdownMenuGroup>
                          <DropdownMenuItem variant="destructive" @select="deleteError = ''; deleteTarget = access"><Trash2Icon />{{ t('admin.smtp.delete') }}</DropdownMenuItem>
                        </DropdownMenuGroup>
                      </DropdownMenuContent>
                    </DropdownMenu>
                  </div>
                </TableCell>
              </TableRow>
            </TableBody>
          </Table>
        </div>
      </template>
    </template>

    <BusyDialog v-model:open="createOpen" :title="t('admin.smtp.createAccess')" :description="t('admin.smtp.passwordGenerated')" :busy="creating">
      <form class="flex flex-col gap-4" @submit.prevent="create">
        <FieldGroup class="gap-4">
          <Field :data-disabled="creating">
            <FieldLabel for="smtp-access-name">{{ t('admin.smtp.name') }}</FieldLabel>
            <Input id="smtp-access-name" v-model="name" :placeholder="t('admin.smtp.laravelPlaceholder')" required maxlength="100" autocomplete="off" :disabled="creating" />
            <FieldDescription>{{ t('admin.smtp.createNameDescription') }}</FieldDescription>
          </Field>
          <Field :data-disabled="creating">
            <FieldLabel for="smtp-access-username">{{ t('admin.smtp.username') }} <span class="font-normal text-muted-foreground">{{ t('admin.smtp.optional') }}</span></FieldLabel>
            <Input id="smtp-access-username" v-model="username" class="font-mono" maxlength="64" autocomplete="off" spellcheck="false" :disabled="creating" />
            <FieldDescription>{{ t('admin.smtp.usernameHint') }}</FieldDescription>
          </Field>
          <SmtpAccessRulesFields
            v-model:requires-encryption="createRequiresEncryption"
            v-model:mechanisms="createMechanisms"
            id-prefix="smtp-access-create"
            :disabled="creating"
            :encryption-available="encryptionAvailable"
          />
          <FieldError v-if="createError" role="alert">{{ createError }}</FieldError>
        </FieldGroup>
        <DialogFooter>
          <Button variant="outline" type="button" :disabled="creating" @click="createOpen = false">{{ t('admin.smtp.cancel') }}</Button>
          <Button type="submit" :disabled="creating || !name.trim() || !createMechanisms.length">
            <Spinner v-if="creating" data-icon="inline-start" aria-hidden="true" />{{ creating ? t('admin.smtp.creating') : t('admin.smtp.createAccess') }}
          </Button>
        </DialogFooter>
      </form>
    </BusyDialog>

    <BusyDialog
      :open="Boolean(regenerateTarget)"
      :title="t('admin.smtp.passwordConfirmTitle', { name: regenerateTarget?.name ?? t('admin.smtp.thisAccess') })"
      :description="t('admin.smtp.passwordConfirmDescription')"
      :busy="regenerating"
      @update:open="!$event && (regenerateTarget = null)"
    >
      <FieldError v-if="regenerateError" role="alert">{{ regenerateError }}</FieldError>
      <DialogFooter>
        <Button variant="outline" :disabled="regenerating" @click="regenerateTarget = null">{{ t('admin.smtp.cancel') }}</Button>
        <Button :disabled="regenerating" @click="regenerate">
          <Spinner v-if="regenerating" data-icon="inline-start" aria-hidden="true" />{{ regenerating ? t('admin.smtp.regenerating') : t('admin.smtp.regenerate') }}
        </Button>
      </DialogFooter>
    </BusyDialog>

    <BusyDialog
      :open="Boolean(deleteTarget)"
      :title="t('admin.smtp.deleteTitle', { name: deleteTarget?.name ?? t('admin.smtp.access') })"
      :description="t('admin.smtp.deleteDescription')"
      :busy="deleting"
      @update:open="!$event && (deleteTarget = null)"
    >
      <FieldError v-if="deleteError" role="alert">{{ deleteError }}</FieldError>
      <DialogFooter>
        <Button variant="outline" :disabled="deleting" @click="deleteTarget = null">{{ t('admin.smtp.cancel') }}</Button>
        <Button variant="destructive" :disabled="deleting" @click="remove">
          <Spinner v-if="deleting" data-icon="inline-start" aria-hidden="true" />{{ deleting ? t('admin.users.deleting') : t('admin.smtp.deleteAccess') }}
        </Button>
      </DialogFooter>
    </BusyDialog>

    <BusyDialog
      :open="Boolean(rulesTarget)"
      :title="t('admin.smtp.authTitle', { name: rulesTarget?.name ?? t('admin.smtp.thisAccess') })"
      :description="t('admin.smtp.authDescription')"
      :busy="savingRules"
      @update:open="!$event && (rulesTarget = null)"
    >
      <form class="flex flex-col gap-4" @submit.prevent="saveRules">
        <SmtpAccessRulesFields
          v-model:requires-encryption="rulesRequiresEncryption"
          v-model:mechanisms="rulesMechanisms"
          id-prefix="smtp-access-rules"
          :disabled="savingRules"
          :encryption-available="encryptionAvailable"
        />
        <FieldError v-if="rulesError" role="alert">{{ rulesError }}</FieldError>
        <DialogFooter>
          <Button variant="outline" type="button" :disabled="savingRules" @click="rulesTarget = null">{{ t('admin.smtp.cancel') }}</Button>
          <Button type="submit" :disabled="savingRules || !rulesChanged || !rulesMechanisms.length">
            <Spinner v-if="savingRules" data-icon="inline-start" aria-hidden="true" />{{ savingRules ? t('admin.users.saving') : t('admin.scopes.save') }}
          </Button>
        </DialogFooter>
      </form>
    </BusyDialog>

    <SmtpCredentialsDialog :credentials="issued" :regenerated="regenerated" @closed="credentialsClosed" />
  </div>
</template>
