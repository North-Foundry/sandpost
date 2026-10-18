<script setup lang="ts">
import { useI18n } from 'vue-i18n'
import { computed, ref } from 'vue'
import { useRoute, useRouter } from 'vue-router'
import {
  AtSignIcon, CheckIcon, CopyIcon, EllipsisIcon, FolderIcon, FolderSearchIcon, InboxIcon, PencilIcon, PlusIcon, RotateCwIcon, Trash2Icon,
} from '@lucide/vue'
import type { SavedView, WebFolder } from '@/api/contracts'
import { describeError } from '@/utils/errors'
import { Alert, AlertDescription } from '@/components/ui/alert'
import { Button } from '@/components/ui/button'
import { DialogFooter } from '@/components/ui/dialog'
import {
  DropdownMenu, DropdownMenuContent, DropdownMenuGroup, DropdownMenuItem, DropdownMenuLabel, DropdownMenuSeparator, DropdownMenuTrigger,
} from '@/components/ui/dropdown-menu'
import {
  SidebarGroup, SidebarGroupAction, SidebarGroupContent, SidebarGroupLabel, SidebarMenu, SidebarMenuAction, SidebarMenuItem,
  SidebarMenuSkeleton,
} from '@/components/ui/sidebar'
import { Spinner } from '@/components/ui/spinner'
import BusyDialog from '@/components/BusyDialog.vue'
import AddressInboxDialog from '@/features/mail/AddressInboxDialog.vue'
import SmartFolderDialog from '@/features/mail/SmartFolderDialog.vue'
import WebFolderDialog from '@/features/mail/WebFolderDialog.vue'
import { folderIcon } from '@/features/mail/folderIcons'
import SidebarNavItem from './SidebarNavItem.vue'
import { DEFAULT_INBOX, TRASH_FOLDER_ID, UNREAD_COUNT_LIMIT, useWorkspace, type MailFolder } from './workspace'
import { useCopy } from '@/composables/useCopy'

const { t, n } = useI18n()

const route = useRoute()
const router = useRouter()
const workspace = useWorkspace()
const addressOpen = ref(false)
const smartOpen = ref(false)
const editingView = ref<SavedView | null>(null)
const editingAddress = ref<string | null>(null)
const editingManualFolder = ref<WebFolder | null>(null)
const manualFolderOpen = ref(false)
const deleteTarget = ref<MailFolder | null>(null)
const deleting = ref(false)
const deleteError = ref('')
const { copied: copiedFolder, error: copyError, copy } = useCopy()
const deleteAllOpen = ref(false)
const deletingAll = ref(false)
const deleteAllError = ref('')
const exposedAccountNames = computed(() => workspace.imapAccounts.value.filter((account) => account.mode === 'mirror').map((account) => account.name))
const deleteAllProgress = ref<{ deleted: number; total: number } | null>(null)

/**
 * Viewers only read mail, so they are not offered deletion.
 */
const canDeleteMail = computed(() => workspace.user.value !== null && workspace.user.value.role !== 'viewer')
const deleteAllLabel = computed(() => {
  const progress = deleteAllProgress.value
  if (!deletingAll.value) return deleteAllError.value ? t('common.tryAgain') : t('common.deleteAll')
  return progress?.total ? t('common.deleteProgress', { deleted: n(progress.deleted), total: n(progress.total) }) : t('common.preparing')
})

const onMail = computed(() => route.name === 'mail' || route.name === 'mail-message')

/**
 * Search results span all mail, so no folder is the current destination while a search is applied.
 */
const browsingFolder = computed(() => onMail.value && typeof route.query.q !== 'string')
const activeFolderId = computed(() => typeof route.query.folder === 'string' ? route.query.folder : DEFAULT_INBOX.id)
const showSkeleton = computed(() => workspace.foldersLoading.value && !workspace.foldersLoaded.value)

/**
 * Inbox unread mail from the shared Seen status; hidden when there is none.
 */
const inboxUnread = computed(() => {
  const count = workspace.inboxUnreadCount.value
  if (!count) return null
  return count >= UNREAD_COUNT_LIMIT ? { badge: '999+', label: t('common.unreadOverflow') } : { badge: String(count), label: t('common.unreadCount', { count: n(count) }) }
})

/**
 * Route to a folder's message list, dropping any applied search.
 */
function folderLocation(folder: MailFolder) {
  return folder.kind === 'inbox' ? { name: 'mail' } : { name: 'mail', query: { folder: folder.id } }
}

/**
 * Report whether the user may change a folder's saved view.
 */
function manageable(folder: MailFolder): boolean {
  if (folder.kind === 'folder') return true
  const view = folder.view
  if (!view) return false
  if (view.ownerId) return view.ownerId === workspace.user.value?.id
  return workspace.canAdminister.value
}

/**
 * Open the manual-folder editor for creation or update.
 */
function editManualFolder(folder: WebFolder | null): void {
  editingManualFolder.value = folder
  manualFolderOpen.value = true
}

/**
 * Open the smart folder editor, empty or for an existing view.
 */
function editSmartFolder(view: SavedView | null, address: string | null = null): void {
  editingView.value = view
  editingAddress.value = address
  smartOpen.value = true
}

/**
 * Copy an address inbox's receiving address and briefly confirm it in the open menu.
 */
async function copyAddress(folder: MailFolder): Promise<void> {
  await copy(folder.address ?? '', folder.id)
}

/**
 * Open the confirmation for deleting every readable email.
 */
function confirmDeleteAll(): void {
  deleteAllError.value = ''
  deleteAllProgress.value = null
  deleteAllOpen.value = true
}

/**
 * Delete every email the user can read. On partial failure the dialog stays open with the reason, and trying again
 * collects whatever is left. An open message that was deleted gives way to its list.
 */
async function deleteAllMail(): Promise<void> {
  if (deletingAll.value) return
  deletingAll.value = true
  deleteAllError.value = ''
  deleteAllProgress.value = null
  try {
    const { failed, total, error } = await workspace.deleteAllMail((deleted, total) => { deleteAllProgress.value = { deleted, total } })
    if (route.name === 'mail-message') await router.push({ name: 'mail', query: route.query })
    if (failed.length) {
      deleteAllError.value = t('common.deletePartial', { failed: n(failed.length), total: n(total), error: describeError(error, '') }).trim()
    } else deleteAllOpen.value = false
  } catch (failure) {
    deleteAllError.value = describeError(failure, t('common.deleteListFailed'))
  } finally { deletingAll.value = false }
}

/**
 * Delete the confirmed folder definition, then leave it if it was open.
 */
async function deleteFolder(): Promise<void> {
  const target = deleteTarget.value
  if (!target || deleting.value) return
  deleting.value = true
  deleteError.value = ''
  try {
    if (target.kind === 'folder') await workspace.deleteManualFolder(target.id)
    else await workspace.deleteView(target.id)
    deleteTarget.value = null
    if (onMail.value && activeFolderId.value === target.id) await router.push({ name: 'mail' })
  } catch (failure) {
    deleteError.value = describeError(failure, t('common.deleteFolderFailed'))
  } finally { deleting.value = false }
}
</script>

<template>
  <SidebarGroup>
    <SidebarGroupLabel>{{ t('common.mail') }}</SidebarGroupLabel>
    <DropdownMenu>
      <DropdownMenuTrigger as-child>
        <SidebarGroupAction :title="t('common.newFolder')" :aria-label="t('common.newFolder')"><PlusIcon /></SidebarGroupAction>
      </DropdownMenuTrigger>
      <DropdownMenuContent side="right" align="start" class="w-64">
        <DropdownMenuGroup>
          <DropdownMenuItem @select="editSmartFolder(null)">
            <FolderSearchIcon />
            <span class="flex flex-col"><span>{{ t('common.smartFolder') }}</span><span class="text-xs text-muted-foreground">{{ t('common.smartFolderHint') }}</span></span>
          </DropdownMenuItem>
          <DropdownMenuItem @select="addressOpen = true">
            <AtSignIcon />
            <span class="flex flex-col"><span>{{ t('common.addressInbox') }}</span><span class="text-xs text-muted-foreground">{{ t('common.addressInboxHint') }}</span></span>
          </DropdownMenuItem>
          <DropdownMenuItem @select="editManualFolder(null)">
            <FolderIcon />
            <span class="flex flex-col"><span>{{ t('common.folder') }}</span><span class="text-xs text-muted-foreground">{{ t('common.folderHint') }}</span></span>
          </DropdownMenuItem>
        </DropdownMenuGroup>
      </DropdownMenuContent>
    </DropdownMenu>

    <SidebarGroupContent>
      <SidebarMenu>
        <template v-if="showSkeleton">
          <SidebarNavItem
            :to="{ name: 'mail' }"
            :icon="InboxIcon"
            :label="t('common.inbox')"
            :badge="inboxUnread?.badge"
            :badge-label="inboxUnread?.label"
            :active="browsingFolder && activeFolderId === DEFAULT_INBOX.id"
          />
          <SidebarNavItem
            :to="{ name: 'mail', query: { folder: TRASH_FOLDER_ID } }"
            :icon="Trash2Icon"
            :label="t('common.trash')"
            :active="browsingFolder && activeFolderId === TRASH_FOLDER_ID"
          />
          <SidebarMenuItem v-for="index in 3" :key="index"><SidebarMenuSkeleton show-icon /></SidebarMenuItem>
        </template>
        <template v-else>
          <SidebarNavItem
            v-for="folder in workspace.folders.value"
            :key="folder.id"
            :to="folderLocation(folder)"
            :icon="folderIcon(folder.view?.icon ?? folder.manual?.icon, folder.kind)"
            :label="folder.name"
            :hint="folder.address ?? undefined"
            :badge="folder.kind === 'inbox' ? inboxUnread?.badge : undefined"
            :badge-label="folder.kind === 'inbox' ? inboxUnread?.label : undefined"
            :active="browsingFolder && activeFolderId === folder.id"
          >
            <template v-if="folder.kind === 'inbox' && canDeleteMail" #action>
              <DropdownMenu>
                <DropdownMenuTrigger as-child>
                  <SidebarMenuAction show-on-hover class="transition-opacity peer-data-[size=default]/menu-button:top-2" :aria-label="t('common.folderActions', { name: folder.name })"><EllipsisIcon /></SidebarMenuAction>
                </DropdownMenuTrigger>
                <DropdownMenuContent side="right" align="start" class="w-52">
                  <DropdownMenuItem variant="destructive" @select="confirmDeleteAll"><Trash2Icon />{{ t('common.deleteAllMenu') }}</DropdownMenuItem>
                </DropdownMenuContent>
              </DropdownMenu>
            </template>
            <template v-else-if="folder.kind !== 'inbox' && folder.kind !== 'trash'" #action>
              <DropdownMenu>
                <DropdownMenuTrigger as-child>
                  <SidebarMenuAction show-on-hover class="transition-opacity peer-data-[size=default]/menu-button:top-2" :aria-label="t('common.folderActions', { name: folder.name })"><EllipsisIcon /></SidebarMenuAction>
                </DropdownMenuTrigger>
                <DropdownMenuContent side="right" align="start" class="w-52">
                  <DropdownMenuLabel v-if="folder.address" class="truncate font-mono text-xs font-normal text-muted-foreground">{{ folder.address }}</DropdownMenuLabel>
                  <DropdownMenuGroup>
                    <DropdownMenuItem v-if="folder.address" @select="(event: Event) => { event.preventDefault(); void copyAddress(folder) }">
                      <CheckIcon v-if="copiedFolder === folder.id" class="text-success! *:text-success!" /><CopyIcon v-else />{{ copiedFolder === folder.id ? t('common.copied') : t('common.copyAddress') }}
                    </DropdownMenuItem>
                    <DropdownMenuItem v-if="folder.kind === 'smart' && manageable(folder)" @select="editSmartFolder(folder.view)"><PencilIcon />{{ t('common.editQuery') }}</DropdownMenuItem>
                    <DropdownMenuItem v-if="folder.kind === 'address' && manageable(folder)" @select="editSmartFolder(folder.view, folder.address)"><PencilIcon />{{ t('common.editFolder') }}</DropdownMenuItem>
                    <DropdownMenuItem v-if="folder.kind === 'folder'" @select="editManualFolder(folder.manual ?? null)"><PencilIcon />{{ t('common.editFolder') }}</DropdownMenuItem>
                  </DropdownMenuGroup>
                  <template v-if="manageable(folder)">
                    <DropdownMenuSeparator />
                    <DropdownMenuGroup>
                      <DropdownMenuItem variant="destructive" @select="deleteError = ''; deleteTarget = folder"><Trash2Icon />{{ t('common.deleteMenu') }}</DropdownMenuItem>
                    </DropdownMenuGroup>
                  </template>
                </DropdownMenuContent>
              </DropdownMenu>
            </template>
          </SidebarNavItem>
        </template>
      </SidebarMenu>
      <div v-if="workspace.foldersError.value" class="flex items-center gap-1 px-2 pt-1 text-xs text-destructive" role="status">
        <span class="min-w-0 flex-1 truncate" :title="workspace.foldersError.value">{{ t('common.foldersUnavailable') }}</span>
        <Button variant="ghost" size="icon-xs" :aria-label="t('common.retryFolders')" @click="workspace.reloadFolders()"><RotateCwIcon /></Button>
      </div>
      <p v-if="copyError" class="px-2 pt-1 text-xs text-destructive" role="status">{{ copyError }}</p>
    </SidebarGroupContent>
  </SidebarGroup>

  <AddressInboxDialog v-model:open="addressOpen" @created="(inbox) => router.push({ name: 'mail', query: { folder: inbox.id } })" />
  <SmartFolderDialog v-model:open="smartOpen" :view="editingView" :address="editingAddress" @saved="(view) => router.push({ name: 'mail', query: { folder: view.id } })" />
  <WebFolderDialog v-model:open="manualFolderOpen" :folder="editingManualFolder" @saved="(folder) => router.push({ name: 'mail', query: { folder: folder.id } })" />
  <BusyDialog
    v-model:open="deleteAllOpen"
    :title="t('common.deleteAllTitle')"
    :description="t('common.deleteAllHint')"
    :busy="deletingAll"
  >
    <Alert v-if="deleteAllError" variant="destructive" role="alert"><AlertDescription>{{ deleteAllError }}</AlertDescription></Alert>
    <DialogFooter>
      <Button variant="outline" :disabled="deletingAll" @click="deleteAllOpen = false">{{ t('common.cancel') }}</Button>
      <Button variant="destructive" :disabled="deletingAll" @click="deleteAllMail">
        <Spinner v-if="deletingAll" data-icon="inline-start" aria-hidden="true" />{{ deleteAllLabel }}
      </Button>
    </DialogFooter>
  </BusyDialog>
  <BusyDialog
    :open="Boolean(deleteTarget)"
    :title="t('common.deleteFolderTitle', { name: deleteTarget?.name ?? t('common.folder') })"
    :description="deleteTarget?.kind === 'folder' ? t('common.deleteManualHint') : t('common.deleteViewHint')"
    :busy="deleting"
    @update:open="!$event && (deleteTarget = null)"
  >
    <template v-if="deleteTarget?.kind === 'folder'">
      <Alert>
        <AlertDescription>
          <template v-if="exposedAccountNames.length">
            <span>{{ t('common.sharingAccounts') }}</span>
            <ul class="my-2 flex list-disc flex-col gap-1 pl-5"><li v-for="name in exposedAccountNames" :key="name">{{ name }}</li></ul>
            <span>{{ t('common.removeSharedFolder') }}</span>
          </template>
          <span v-else>{{ t('common.noSharingAccounts') }}</span>
          {{ t('common.independentHint') }}
        </AlertDescription>
      </Alert>
    </template>
    <Alert v-if="deleteError" variant="destructive" role="alert"><AlertDescription>{{ deleteError }}</AlertDescription></Alert>
    <DialogFooter>
      <Button variant="outline" :disabled="deleting" @click="deleteTarget = null">{{ t('common.cancel') }}</Button>
      <Button variant="destructive" :disabled="deleting" @click="deleteFolder">
        <Spinner v-if="deleting" data-icon="inline-start" aria-hidden="true" />{{ deleting ? t('common.deleting') : t('common.deleteFolder') }}
      </Button>
    </DialogFooter>
  </BusyDialog>
</template>
