<script setup lang="ts">
import { computed, onBeforeUnmount, onMounted, ref, watch } from 'vue'
import { useI18n } from 'vue-i18n'
import { RouterLink, useRoute, useRouter } from 'vue-router'
import {
  ArchiveRestoreIcon, ArrowLeftIcon, CheckIcon, CopyIcon, FolderSearchIcon, InboxIcon, LinkIcon, MailboxIcon, MoreHorizontalIcon,
  PencilIcon, PlusIcon, PowerIcon, PowerOffIcon, RefreshCwIcon, Trash2Icon, XIcon,
} from '@lucide/vue'
import type { ImapAccount, ImapAccountMode, ImapLocalFolder, ImapMailbox, ImapServer } from '@/api/contracts'
import { useSandpostClient } from '@/runtime/client'
import { describeError } from '@/utils/errors'
import { formatFullDate } from '@/utils/format'
import { Alert, AlertAction, AlertDescription } from '@/components/ui/alert'
import { Badge } from '@/components/ui/badge'
import { Button } from '@/components/ui/button'
import { Card, CardAction, CardContent, CardDescription, CardFooter, CardHeader, CardTitle } from '@/components/ui/card'
import { Collapsible, CollapsibleContent, CollapsibleTrigger } from '@/components/ui/collapsible'
import { DialogFooter } from '@/components/ui/dialog'
import {
  DropdownMenu, DropdownMenuContent, DropdownMenuGroup, DropdownMenuItem, DropdownMenuSeparator, DropdownMenuTrigger,
} from '@/components/ui/dropdown-menu'
import { Empty, EmptyContent, EmptyDescription, EmptyHeader, EmptyMedia, EmptyTitle } from '@/components/ui/empty'
import { Field, FieldDescription, FieldError, FieldGroup, FieldLabel } from '@/components/ui/field'
import { Input } from '@/components/ui/input'
import { InputGroup, InputGroupAddon, InputGroupButton, InputGroupInput } from '@/components/ui/input-group'
import { Item, ItemActions, ItemContent, ItemDescription, ItemGroup, ItemMedia, ItemTitle } from '@/components/ui/item'
import { Skeleton } from '@/components/ui/skeleton'
import { Spinner } from '@/components/ui/spinner'
import BusyDialog from '@/components/BusyDialog.vue'
import ImapAccountDialogs from '@/features/imap/ImapAccountDialogs.vue'
import ImapFolderDialog from '@/features/imap/ImapFolderDialog.vue'
import ImapServerNotice from '@/features/imap/ImapServerNotice.vue'
import ImapSetupInstructions from '@/features/imap/ImapSetupInstructions.vue'
import { describeRestoration, useImapAccountActions } from '@/features/imap/accountActions'
import { imapConnectionFields, imapSecurityDescription, type ImapConnectionField } from '@/features/imap/security'
import { useCopy } from '@/composables/useCopy'
import { useWorkspace } from '@/features/shell/workspace'
import { ToggleGroup, ToggleGroupItem } from '@/components/ui/toggle-group'

const client = useSandpostClient()
const { t, locale } = useI18n()
const workspace = useWorkspace()
const canMutate = computed(() => Boolean(workspace.user?.value && workspace.user.value.role !== 'viewer'))
let unsubscribeEvents: (() => void) | null = null
const route = useRoute()
const router = useRouter()
const account = ref<ImapAccount | null>(null)
const server = ref<ImapServer | null>(null)
const loading = ref(true)
const error = ref('')
const notFound = ref(false)

const name = ref('')
const savingName = ref(false)
const nameError = ref('')
const toggling = ref(false)
const statusError = ref('')

const mode = ref<ImapAccountMode>('mirror')
const savingMailboxes = ref(false)
const mailboxesError = ref('')

const folderOpen = ref(false)
const editingFolder = ref<ImapLocalFolder | null>(null)
const folderDeleteTarget = ref<ImapLocalFolder | null>(null)
const deletingFolder = ref(false)
const folderDeleteError = ref('')

const instructionsOpen = ref(false)
const { copied, error: copyError, copy: copyValue } = useCopy<ImapConnectionField['key']>()

const accountId = computed(() => String(route.params.id ?? ''))
const connectionFields = computed(() => {
  locale.value
  return account.value && server.value
    ? imapConnectionFields(server.value, account.value, null).filter((field) => field.key !== 'host' || server.value?.enabled)
    : []
})
const nameDirty = computed(() => Boolean(account.value) && name.value.trim() !== account.value?.name)
const mailboxesDirty = computed(() => {
  if (!account.value) return false
  return mode.value !== account.value.mode
})

/**
 * The Smart Folders exposed as IMAP mailboxes, with the IMAP name each one uses.
 */
const linkedMailboxes = computed(() => (account.value?.mailboxes ?? []).filter((mailbox) => mailbox.source === 'view'))
const sortedFolders = computed(() => [...(account.value?.folders ?? [])].filter((folder) => folder.kind === 'smart').sort((first, second) => first.name.localeCompare(second.name)))

/**
 * The IMAP mailbox name of each local folder, which may carry a collision suffix.
 */
const folderMailboxNames = computed(() => new Map((account.value?.mailboxes ?? [])
  .filter((mailbox) => mailbox.source === 'folder' && mailbox.folderId)
  .map((mailbox) => [mailbox.folderId!, mailbox.name])))

const actions = useImapAccountActions(client, {
  /**
   * Refresh the displayed account only when the changed record is the one being edited.
   */
  changed: (updated) => { if (updated.id === account.value?.id) show(updated) },

  /**
   * Leave the editor after its account has been deleted.
   */
  deleted: async () => { await router.replace({ name: 'imap' }) },
})

/**
 * Show the server's representation and reset the editors that have no unsaved changes.
 */
function show(updated: ImapAccount, resetEditors = true): void {
  const previous = account.value
  account.value = updated
  if (resetEditors || !previous || name.value.trim() === previous.name) name.value = updated.name
  if (resetEditors || !previous || !mailboxesDirtyAgainst(previous)) {
    mode.value = updated.mode
  }
}

/**
 * Report whether the mode editor differs from a given saved account.
 */
function mailboxesDirtyAgainst(saved: ImapAccount): boolean {
  return mode.value !== saved.mode
}

/**
 * Describe where an exposed mailbox takes its messages from.
 */
function mailboxSource(mailbox: ImapMailbox): string {
  switch (mailbox.source) {
    case 'inbox': return account.value?.mode === 'mirror' ? t('imap.mailboxSource.allReadable') : t('imap.mailboxSource.independentMail')
    case 'trash': return t('imap.mailboxSource.trash')
    case 'view': return t('imap.mailboxSource.smartFolder')
    case 'folder': return t('imap.mailboxSource.localFolder')
  }
}

/**
 * Load the account and the IMAP service details.
 */
async function load(): Promise<void> {
  loading.value = true
  error.value = ''
  notFound.value = false
  try {
    const [loadedServer, loadedAccount] = await Promise.all([client.imap.server(), client.imap.get(accountId.value)])
    server.value = loadedServer
    show(loadedAccount)
  } catch (failure) {
    const message = describeError(failure, t('imap.errors.loadAccount'))
    if (/not found/i.test(message)) notFound.value = true
    else error.value = message
  } finally {
    loading.value = false
  }
}

/**
 * Refresh the exposed mailboxes after Smart Folders change, keeping unsaved edits.
 */
async function refreshQuietly(): Promise<void> {
  if (!account.value) return
  try {
    show(await client.imap.get(account.value.id), false)
  } catch { /*
   * The previous preview stays visible; the next explicit action reports errors.
   */ }
}

/**
 * Rename the account; mail clients keep working because the username does not change.
 */
async function saveName(): Promise<void> {
  if (!canMutate.value || !account.value || savingName.value || !nameDirty.value || !name.value.trim()) return
  savingName.value = true
  nameError.value = ''
  try {
    show(await client.imap.update(account.value.id, { name: name.value.trim() }), false)
    name.value = account.value.name
  } catch (failure) {
    nameError.value = describeError(failure, t('imap.errors.renameAccount'))
  } finally {
    savingName.value = false
  }
}

/**
 * Enable or disable the account; disabling signs its mail clients out at once.
 */
async function toggle(): Promise<void> {
  if (!canMutate.value || !account.value || toggling.value) return
  toggling.value = true
  statusError.value = ''
  try {
    show(await client.imap.update(account.value.id, { enabled: !account.value.enabled }), false)
  } catch (failure) {
    statusError.value = describeError(failure, t('imap.errors.updateAccount'))
  } finally {
    toggling.value = false
  }
}

/**
 * Save the account mode while preserving its local folder state.
 */
async function saveMailboxes(): Promise<void> {
  if (!canMutate.value || !account.value || savingMailboxes.value || !mailboxesDirty.value) return
  savingMailboxes.value = true
  mailboxesError.value = ''
  try {
    const updated = await client.imap.update(account.value.id, { mode: mode.value })
    show(updated)
  } catch (failure) {
    mailboxesError.value = describeError(failure, t('imap.errors.saveMode'))
  } finally {
    savingMailboxes.value = false
  }
}

/**
 * Return the mailbox editor to the saved selection.
 */
function resetMailboxes(): void {
  if (!account.value) return
  mode.value = account.value.mode
  mailboxesError.value = ''
}

/**
 * Open the local folder editor, empty or for an existing folder.
 */
function editFolder(folder: ImapLocalFolder | null): void {
  if (!canMutate.value || account.value?.mode !== 'independent' || folder && folder.kind !== 'smart') return
  editingFolder.value = folder
  folderOpen.value = true
}

/**
 * Delete the confirmed local folder; Sandpost mail and Smart Folders are untouched.
 */
async function deleteFolder(): Promise<void> {
  const target = folderDeleteTarget.value
  if (!canMutate.value || !account.value || !target || account.value.mode !== 'independent' || target.kind !== 'smart' || deletingFolder.value) return
  deletingFolder.value = true
  folderDeleteError.value = ''
  try {
    await client.imap.folders.delete(account.value.id, target.id)
    folderDeleteTarget.value = null
    await refreshQuietly()
  } catch (failure) {
    folderDeleteError.value = describeError(failure, t('imap.errors.deleteFolder'))
  } finally {
    deletingFolder.value = false
  }
}

/**
 * Copy one connection value and briefly confirm it inline.
 */
async function copy(field: ImapConnectionField): Promise<void> {
  await copyValue(field.value, field.key)
}

watch(() => account.value?.mode, (mode) => {
  if (mode !== 'independent') { folderOpen.value = false; folderDeleteTarget.value = null }
})
watch(accountId, (id, previous) => { if (id && id !== previous) void load() })

/**
 * Ignore attempts to deselect the saved mode.
 */
function setMode(value: unknown): void {
  if (!canMutate.value) return
  if (value === 'mirror' || value === 'independent') mode.value = value
}
onMounted(() => {
  void load()
  unsubscribeEvents = workspace.onMailEvent((event) => { if (event.type !== 'disconnected') void refreshQuietly() })
})
onBeforeUnmount(() => unsubscribeEvents?.())
</script>

<template>
  <section class="h-full overflow-y-auto" aria-labelledby="imap-account-title">
    <div class="mx-auto flex max-w-4xl flex-col gap-6 p-4 sm:p-6">
      <Button as-child variant="ghost" size="sm" class="w-fit">
        <RouterLink :to="{ name: 'imap' }"><ArrowLeftIcon data-icon="inline-start" />{{ t('imap.accounts.title') }}</RouterLink>
      </Button>

      <div v-if="loading" class="flex flex-col gap-4">
        <Skeleton class="h-7 w-56" />
        <Skeleton class="h-40 w-full" />
        <Skeleton class="h-56 w-full" />
      </div>
      <Empty v-else-if="notFound" class="border">
        <EmptyHeader>
          <EmptyMedia variant="icon"><MailboxIcon /></EmptyMedia>
          <EmptyTitle>{{ t('imap.account.notFoundTitle') }}</EmptyTitle>
          <EmptyDescription>{{ t('imap.account.notFoundDescription') }}</EmptyDescription>
        </EmptyHeader>
        <EmptyContent><Button as-child variant="outline"><RouterLink :to="{ name: 'imap' }">{{ t('imap.account.backToAccounts') }}</RouterLink></Button></EmptyContent>
      </Empty>
      <Alert v-else-if="error" variant="destructive" role="alert">
        <AlertDescription class="flex items-center gap-3"><span class="flex-1">{{ error }}</span><Button variant="outline" size="sm" @click="load">{{ t('imap.common.retry') }}</Button></AlertDescription>
      </Alert>
      <template v-else-if="account && server">
        <div class="flex flex-wrap items-start justify-between gap-3">
          <div class="flex min-w-0 flex-col gap-1">
            <div class="flex items-center gap-2">
              <h1 id="imap-account-title" class="truncate text-lg font-semibold tracking-tight">{{ account.name }}</h1>
              <Badge :variant="account.enabled ? 'secondary' : 'outline'">{{ account.enabled ? t('imap.accounts.active') : t('imap.accounts.disabled') }}</Badge>
            </div>
            <p class="text-sm text-muted-foreground">
              {{ t('imap.account.createdAt', { date: formatFullDate(account.createdAt) }) }} · {{ account.lastUsedAt ? t('imap.account.lastUsedAt', { date: formatFullDate(account.lastUsedAt) }) : t('imap.account.neverUsed') }}
            </p>
          </div>
          <DropdownMenu>
            <DropdownMenuTrigger as-child>
              <Button v-if="canMutate" variant="outline" size="icon-sm" :aria-label="t('imap.account.moreActions')"><MoreHorizontalIcon /></Button>
            </DropdownMenuTrigger>
            <DropdownMenuContent align="end" class="w-56">
              <DropdownMenuGroup>
                <DropdownMenuItem @select="actions.confirmRegenerate(account)"><RefreshCwIcon />{{ t('imap.dialogs.regenerate') }}…</DropdownMenuItem>
                <DropdownMenuItem @select="actions.confirmRestore(account)"><ArchiveRestoreIcon />{{ t('imap.dialogs.restore') }}…</DropdownMenuItem>
                <DropdownMenuItem @select="toggle"><component :is="account.enabled ? PowerOffIcon : PowerIcon" />{{ account.enabled ? t('imap.accounts.disable') : t('imap.accounts.enable') }}</DropdownMenuItem>
              </DropdownMenuGroup>
              <DropdownMenuSeparator />
              <DropdownMenuGroup>
                <DropdownMenuItem variant="destructive" @select="actions.confirmDelete(account)"><Trash2Icon />{{ t('imap.dialogs.deleteAccount') }}…</DropdownMenuItem>
              </DropdownMenuGroup>
            </DropdownMenuContent>
          </DropdownMenu>
        </div>

        <ImapServerNotice :server="server" />
        <Alert v-if="!account.enabled" role="status">
          <PowerOffIcon />
          <AlertDescription>{{ t('imap.account.disabledDescription') }}</AlertDescription>
        </Alert>
        <Alert v-if="actions.restoreResult.value" role="status">
          <ArchiveRestoreIcon />
          <AlertDescription>{{ describeRestoration(actions.restoreResult.value.restored) }}</AlertDescription>
          <AlertAction>
            <Button variant="ghost" size="icon-xs" :aria-label="t('imap.common.dismiss')" @click="actions.restoreResult.value = null"><XIcon /></Button>
          </AlertAction>
        </Alert>

        <Card>
          <CardHeader>
            <CardTitle>{{ t('imap.account.detailsTitle') }}</CardTitle>
            <CardDescription>{{ t('imap.account.nameDescription') }}</CardDescription>
          </CardHeader>
          <CardContent>
            <form id="imap-account-name-form" @submit.prevent="saveName">
              <FieldGroup class="gap-4">
                <Field :data-disabled="savingName" :data-invalid="Boolean(nameError)">
                  <FieldLabel for="imap-account-edit-name">{{ t('imap.common.name') }}</FieldLabel>
                  <Input id="imap-account-edit-name" v-model="name" required maxlength="100" autocomplete="off" :disabled="!canMutate || savingName" :aria-invalid="Boolean(nameError)" />
                  <FieldError v-if="nameError" role="alert">{{ nameError }}</FieldError>
                </Field>
                <FieldError v-if="statusError" role="alert">{{ statusError }}</FieldError>
              </FieldGroup>
            </form>
          </CardContent>
          <CardFooter class="flex flex-wrap justify-end gap-2 border-t">
              <Button v-if="canMutate" variant="outline" type="button" :disabled="toggling" @click="toggle">
              <Spinner v-if="toggling" data-icon="inline-start" aria-hidden="true" />
              <component :is="account.enabled ? PowerOffIcon : PowerIcon" v-else data-icon="inline-start" />{{ account.enabled ? t('imap.account.disableAccount') : t('imap.account.enableAccount') }}
            </Button>
            <Button v-if="canMutate" type="submit" form="imap-account-name-form" :disabled="savingName || !nameDirty || !name.trim()">
              <Spinner v-if="savingName" data-icon="inline-start" aria-hidden="true" />{{ savingName ? t('imap.common.saving') : t('imap.account.saveName') }}
            </Button>
          </CardFooter>
        </Card>

        <Card>
          <CardHeader>
            <CardTitle>{{ t('imap.account.connectionTitle') }}</CardTitle>
            <CardDescription>{{ server.enabled ? imapSecurityDescription(server.security) : t('imap.account.connectionDisabled') }}</CardDescription>
            <CardAction>
              <Button v-if="canMutate" variant="outline" size="sm" @click="actions.confirmRegenerate(account)"><RefreshCwIcon data-icon="inline-start" />{{ t('imap.dialogs.regenerate') }}</Button>
            </CardAction>
          </CardHeader>
          <CardContent class="flex flex-col gap-4">
            <FieldGroup class="grid gap-3 sm:grid-cols-2">
              <Field v-for="field in connectionFields" :key="field.key">
                <FieldLabel :for="`imap-connection-${field.key}`">{{ field.label }}</FieldLabel>
                <InputGroup>
                  <InputGroupInput
                    :id="`imap-connection-${field.key}`"
                    :model-value="field.value"
                    readonly
                    class="font-mono"
                    autocomplete="off"
                    spellcheck="false"
                    :aria-describedby="field.key === 'password' ? 'imap-connection-password-hint' : undefined"
                    @focus="($event.target as HTMLInputElement).select()"
                  />
                  <InputGroupAddon v-if="field.key !== 'password'" align="inline-end">
                    <InputGroupButton size="icon-xs" :aria-label="t('imap.credentials.copyField', { field: field.label.toLowerCase() })" @click="copy(field)">
                      <CheckIcon v-if="copied === field.key" class="text-success" />
                      <CopyIcon v-else />
                    </InputGroupButton>
                  </InputGroupAddon>
                </InputGroup>
                <FieldDescription v-if="field.key === 'password'" id="imap-connection-password-hint">{{ t('imap.account.passwordDescription') }}</FieldDescription>
              </Field>
            </FieldGroup>
            <FieldError v-if="copyError" role="alert">{{ copyError }}</FieldError>
            <Collapsible v-model:open="instructionsOpen" class="flex flex-col gap-3">
              <CollapsibleTrigger as-child>
                <Button variant="ghost" size="sm" class="w-fit">{{ instructionsOpen ? t('imap.account.hideInstructions') : t('imap.account.showInstructions') }}</Button>
              </CollapsibleTrigger>
              <CollapsibleContent class="flex flex-col gap-3">
                <ImapSetupInstructions :server="server" :username="account.username" />
              </CollapsibleContent>
            </Collapsible>
          </CardContent>
        </Card>

        <Card>
          <CardHeader>
            <CardTitle>{{ t('imap.account.mode') }}</CardTitle>
            <CardDescription>
              {{ t('imap.account.modeSecurityDescription') }}
            </CardDescription>
          </CardHeader>
          <CardContent>
            <form id="imap-mailboxes-form" class="flex flex-col gap-4" @submit.prevent="saveMailboxes">
              <FieldGroup class="gap-4">
                <Field :data-disabled="savingMailboxes">
                  <FieldLabel>{{ t('imap.account.mode') }}</FieldLabel>
                  <ToggleGroup v-if="canMutate" :model-value="mode" @update:model-value="setMode" type="single" variant="outline" :disabled="savingMailboxes" :aria-label="t('imap.account.mode')">
                    <ToggleGroupItem value="mirror">{{ t('imap.account.mirror') }}</ToggleGroupItem>
                    <ToggleGroupItem value="independent">{{ t('imap.account.independent') }}</ToggleGroupItem>
                  </ToggleGroup>
                  <FieldDescription>
                    {{ t('imap.account.modeDescription') }}
                  </FieldDescription>
                </Field>
                <FieldError v-if="mailboxesError" role="alert">{{ mailboxesError }}</FieldError>
              </FieldGroup>
            </form>
          </CardContent>
          <CardFooter class="flex justify-end gap-2 border-t">
            <Button v-if="canMutate && mailboxesDirty" variant="ghost" type="button" :disabled="savingMailboxes" @click="resetMailboxes">{{ t('imap.common.discardChanges') }}</Button>
            <Button v-if="canMutate" type="submit" form="imap-mailboxes-form" :disabled="savingMailboxes || !mailboxesDirty">
              <Spinner v-if="savingMailboxes" data-icon="inline-start" aria-hidden="true" />{{ savingMailboxes ? t('imap.common.saving') : t('imap.account.saveMode') }}
            </Button>
          </CardFooter>
        </Card>

        <Card v-if="account.mode === 'mirror'">
          <CardHeader>
              <CardTitle class="flex items-center gap-2"><LinkIcon class="size-4 text-muted-foreground" aria-hidden="true" />{{ t('imap.account.linkedFolders') }}</CardTitle>
            <CardDescription>{{ t('imap.account.linkedFoldersDescription') }}</CardDescription>
          </CardHeader>
          <CardContent>
            <p v-if="!linkedMailboxes.length" class="text-sm text-muted-foreground">
              {{ t('imap.account.noLinkedFolders') }}
            </p>
            <ItemGroup v-else class="gap-1">
              <Item v-for="mailbox in linkedMailboxes" :key="mailbox.viewId ?? mailbox.name" size="xs" variant="muted">
                <ItemMedia variant="icon"><FolderSearchIcon /></ItemMedia>
                <ItemContent class="min-w-0">
                  <ItemTitle class="truncate font-mono">{{ mailbox.name }}</ItemTitle>
                </ItemContent>
              </Item>
            </ItemGroup>
          </CardContent>
        </Card>

        <Card v-if="account.mode === 'independent'">
          <CardHeader>
            <CardTitle class="flex items-center gap-2"><MailboxIcon class="size-4 text-muted-foreground" aria-hidden="true" />{{ t('imap.account.independentFolders') }}</CardTitle>
            <CardDescription>{{ t('imap.account.independentFoldersDescription') }}</CardDescription>
            <CardAction>
              <Button v-if="canMutate" variant="outline" size="sm" @click="editFolder(null)"><PlusIcon data-icon="inline-start" />{{ t('imap.account.addSmartFolder') }}</Button>
            </CardAction>
          </CardHeader>
          <CardContent>
            <p v-if="!sortedFolders.length" class="text-sm text-muted-foreground">{{ t('imap.account.noIndependentFolders') }}</p>
            <ItemGroup v-else class="gap-1">
              <Item v-for="folder in sortedFolders" :key="folder.id" size="xs" variant="muted">
                <ItemContent class="min-w-0">
                  <ItemTitle class="flex min-w-0 items-center gap-2">
                    <span class="truncate font-mono">{{ folderMailboxNames.get(folder.id) ?? folder.name }}</span>
                  </ItemTitle>
                  <ItemDescription class="truncate font-mono" :title="folder.filter">{{ folder.filter || t('imap.mailboxSource.allReadable') }}</ItemDescription>
                </ItemContent>
                <ItemActions v-if="canMutate">
                  <Button variant="ghost" size="icon-sm" :aria-label="t('imap.account.editFolder', { folder: folder.name })" @click="editFolder(folder)"><PencilIcon /></Button>
                  <Button variant="ghost" size="icon-sm" :aria-label="t('imap.account.deleteFolder', { folder: folder.name })" @click="folderDeleteError = ''; folderDeleteTarget = folder"><Trash2Icon /></Button>
                </ItemActions>
              </Item>
            </ItemGroup>
          </CardContent>
        </Card>

        <Card>
          <CardHeader>
            <CardTitle class="flex items-center gap-2"><InboxIcon class="size-4 text-muted-foreground" aria-hidden="true" />{{ t('imap.account.exposedMailboxes') }}</CardTitle>
            <CardDescription>{{ t('imap.account.exposedMailboxesDescription') }}</CardDescription>
          </CardHeader>
          <CardContent>
            <ul class="flex flex-col divide-y rounded-lg border" :aria-label="t('imap.account.exposedMailboxes')">
              <li v-for="mailbox in account.mailboxes" :key="`${mailbox.source}:${mailbox.name}`" class="flex min-w-0 flex-wrap items-baseline justify-between gap-x-3 gap-y-0.5 px-3 py-2 text-sm">
                <span class="truncate font-mono">{{ mailbox.name }}</span>
                <span class="truncate text-xs text-muted-foreground">{{ mailboxSource(mailbox) }}</span>
              </li>
            </ul>
          </CardContent>
        </Card>

        <Card>
          <CardHeader>
            <CardTitle>{{ t('imap.account.deletingTitle') }}</CardTitle>
            <CardDescription>
              {{ account.mode === 'mirror'
                ? t('imap.account.deletingMirrorDescription')
                : t('imap.account.deletingIndependentDescription') }}
            </CardDescription>
          </CardHeader>
          <CardFooter v-if="canMutate" class="flex flex-wrap justify-end gap-2 border-t">
            <Button variant="outline" @click="actions.confirmRestore(account)"><ArchiveRestoreIcon data-icon="inline-start" />{{ t('imap.dialogs.restore') }}</Button>
            <Button variant="destructive" @click="actions.confirmDelete(account)"><Trash2Icon data-icon="inline-start" />{{ t('imap.dialogs.deleteAccount') }}</Button>
          </CardFooter>
        </Card>
      </template>
    </div>

    <ImapFolderDialog
      v-if="canMutate && account?.mode === 'independent'"
      v-model:open="folderOpen"
      :account-id="account.id"
      :folder="editingFolder"
      @saved="refreshQuietly"
    />
    <BusyDialog v-if="canMutate"
      :open="Boolean(folderDeleteTarget)"
      :title="t('imap.account.deleteFolderTitle', { folder: folderDeleteTarget?.name ?? t('imap.account.folder') })"
      :description="t('imap.account.deleteFolderDescription')"
      :busy="deletingFolder"
      @update:open="!$event && (folderDeleteTarget = null)"
    >
      <FieldError v-if="folderDeleteError" role="alert">{{ folderDeleteError }}</FieldError>
      <DialogFooter>
        <Button variant="outline" :disabled="deletingFolder" @click="folderDeleteTarget = null">{{ t('imap.common.cancel') }}</Button>
        <Button variant="destructive" :disabled="deletingFolder" @click="deleteFolder">
          <Spinner v-if="deletingFolder" data-icon="inline-start" aria-hidden="true" />{{ deletingFolder ? t('imap.common.deleting') : t('imap.account.deleteFolderAction') }}
        </Button>
      </DialogFooter>
    </BusyDialog>
    <ImapAccountDialogs v-if="canMutate" :actions="actions" />
  </section>
</template>
