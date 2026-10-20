<script setup lang="ts">
import { useI18n } from 'vue-i18n'
import { computed, ref } from 'vue'
import { useRoute, useRouter } from 'vue-router'
import {
  ActivityIcon, CheckIcon, CirclePauseIcon, CirclePlayIcon, CopyIcon, EllipsisIcon, PencilIcon, PlusIcon, Trash2Icon, VenetianMaskIcon,
} from '@lucide/vue'
import type { Persona } from '@/api/contracts'
import { describeError } from '@/utils/errors'
import { Alert, AlertDescription } from '@/components/ui/alert'
import { Button } from '@/components/ui/button'
import { DialogFooter } from '@/components/ui/dialog'
import {
  DropdownMenu, DropdownMenuContent, DropdownMenuGroup, DropdownMenuItem, DropdownMenuLabel, DropdownMenuSeparator, DropdownMenuTrigger,
} from '@/components/ui/dropdown-menu'
import {
  SidebarGroup, SidebarGroupAction, SidebarGroupContent, SidebarGroupLabel, SidebarMenu, SidebarMenuAction, SidebarMenuButton, SidebarMenuItem,
} from '@/components/ui/sidebar'
import { Spinner } from '@/components/ui/spinner'
import BusyDialog from '@/components/BusyDialog.vue'
import PersonaActivitySheet from '@/features/personas/PersonaActivitySheet.vue'
import PersonaDialog from '@/features/personas/PersonaDialog.vue'
import { useCopy } from '@/composables/useCopy'
import SidebarNavItem from './SidebarNavItem.vue'
import { useWorkspace } from './workspace'

const { t } = useI18n()
const route = useRoute()
const router = useRouter()
const workspace = useWorkspace()
const dialogOpen = ref(false)
const editing = ref<Persona | null>(null)
const activityOpen = ref(false)
const activityPersona = ref<Persona | null>(null)
const deleteTarget = ref<Persona | null>(null)
const deleting = ref(false)
const deleteError = ref('')
const toggleError = ref('')
const { copied, error: copyError, copy } = useCopy()

/**
 * Viewers read mail but cannot create or change Personas.
 */
const canManage = computed(() => workspace.user.value !== null && workspace.user.value.role !== 'viewer')
const onMail = computed(() => route.name === 'mail' || route.name === 'mail-message')
const activeFolderId = computed(() => onMail.value && typeof route.query.q !== 'string' && typeof route.query.folder === 'string' ? route.query.folder : null)

/**
 * Open the editor for a new or existing Persona.
 */
function edit(persona: Persona | null): void {
  editing.value = persona
  dialogOpen.value = true
}

/**
 * Show a Persona's run history.
 */
function showActivity(persona: Persona): void {
  activityPersona.value = persona
  activityOpen.value = true
}

/**
 * Enable or disable a Persona from its menu.
 */
async function toggle(persona: Persona): Promise<void> {
  toggleError.value = ''
  try {
    await workspace.setPersonaEnabled(persona.id, !persona.enabled)
  } catch (failure) {
    toggleError.value = describeError(failure, t('personas.toggleFailed'))
  }
}

/**
 * Delete the confirmed Persona, leaving its view if it was open.
 */
async function deletePersona(): Promise<void> {
  const target = deleteTarget.value
  if (!target || deleting.value) return
  deleting.value = true
  deleteError.value = ''
  try {
    await workspace.deletePersona(target.id)
    deleteTarget.value = null
    if (activeFolderId.value === target.id) await router.push({ name: 'mail' })
  } catch (failure) {
    deleteError.value = describeError(failure, t('personas.deleteFailed'))
  } finally { deleting.value = false }
}

/**
 * Open a message from the activity sheet inside the Persona's view.
 */
function openMessage(persona: Persona | null, id: string): void {
  void router.push({ name: 'mail-message', params: { id }, query: persona ? { folder: persona.id } : {} })
}
</script>

<template>
  <SidebarGroup v-if="workspace.personasAvailable">
    <SidebarGroupLabel>{{ t('personas.section') }}</SidebarGroupLabel>
    <SidebarGroupAction v-if="canManage" :title="t('personas.new')" :aria-label="t('personas.new')" @click="edit(null)"><PlusIcon /></SidebarGroupAction>
    <SidebarGroupContent>
      <SidebarMenu>
        <SidebarNavItem
          v-for="folder in workspace.personaFolders.value"
          :key="folder.id"
          :to="{ name: 'mail', query: { folder: folder.id } }"
          :icon="folder.persona?.enabled === false ? CirclePauseIcon : VenetianMaskIcon"
          :label="folder.name"
          :hint="folder.persona?.enabled === false ? `${folder.address} · ${t('personas.disabled')}` : folder.address ?? undefined"
          :active="activeFolderId === folder.id"
        >
          <template v-if="folder.persona" #action>
            <DropdownMenu>
              <DropdownMenuTrigger as-child>
                <SidebarMenuAction show-on-hover class="transition-opacity peer-data-[size=default]/menu-button:top-2" :aria-label="t('personas.actions', { name: folder.name })"><EllipsisIcon /></SidebarMenuAction>
              </DropdownMenuTrigger>
              <DropdownMenuContent side="right" align="start" class="w-56">
                <DropdownMenuLabel class="truncate font-mono text-xs font-normal text-muted-foreground">{{ folder.address }}</DropdownMenuLabel>
                <DropdownMenuGroup>
                  <DropdownMenuItem @select="(event: Event) => { event.preventDefault(); void copy(folder.address ?? '', folder.id) }">
                    <CheckIcon v-if="copied === folder.id" class="text-success! *:text-success!" /><CopyIcon v-else />{{ copied === folder.id ? t('personas.copied') : t('personas.copyAddress') }}
                  </DropdownMenuItem>
                  <DropdownMenuItem @select="showActivity(folder.persona)"><ActivityIcon />{{ t('personas.activity.button') }}</DropdownMenuItem>
                  <template v-if="canManage && folder.persona.ownerId === workspace.user.value?.id">
                    <DropdownMenuItem @select="edit(folder.persona)"><PencilIcon />{{ t('personas.edit') }}</DropdownMenuItem>
                    <DropdownMenuItem @select="toggle(folder.persona)">
                      <template v-if="folder.persona.enabled"><CirclePauseIcon />{{ t('personas.disable') }}</template>
                      <template v-else><CirclePlayIcon />{{ t('personas.enable') }}</template>
                    </DropdownMenuItem>
                  </template>
                </DropdownMenuGroup>
                <template v-if="canManage">
                  <DropdownMenuSeparator />
                  <DropdownMenuItem variant="destructive" @select="deleteError = ''; deleteTarget = folder.persona"><Trash2Icon />{{ t('personas.delete') }}</DropdownMenuItem>
                </template>
              </DropdownMenuContent>
            </DropdownMenu>
          </template>
        </SidebarNavItem>
        <SidebarMenuItem v-if="canManage && workspace.foldersLoaded.value && !workspace.personaFolders.value.length">
          <SidebarMenuButton class="h-auto py-1.5 text-muted-foreground [&_svg]:size-5" :tooltip="t('personas.new')" @click="edit(null)">
            <PlusIcon aria-hidden="true" />
            <span class="flex min-w-0 flex-col text-left group-data-[collapsible=icon]:sr-only">
              <span class="text-sm text-sidebar-foreground">{{ t('personas.new') }}</span>
              <span class="text-xs">{{ t('personas.newHint') }}</span>
            </span>
          </SidebarMenuButton>
        </SidebarMenuItem>
      </SidebarMenu>
      <p v-if="copyError || toggleError" class="px-2 pt-1 text-xs text-destructive" role="status">{{ copyError || toggleError }}</p>
    </SidebarGroupContent>
  </SidebarGroup>

  <PersonaDialog v-if="workspace.personasAvailable" v-model:open="dialogOpen" :persona="editing" @saved="(persona) => router.push({ name: 'mail', query: { folder: persona.id } })" />
  <PersonaActivitySheet v-if="workspace.personasAvailable" v-model:open="activityOpen" :persona="activityPersona" @open-message="(id) => openMessage(activityPersona, id)" />
  <BusyDialog
    :open="Boolean(deleteTarget)"
    :title="t('personas.deleteTitle', { name: deleteTarget?.name ?? '' })"
    :description="t('personas.deleteHint')"
    :busy="deleting"
    @update:open="!$event && (deleteTarget = null)"
  >
    <Alert v-if="deleteError" variant="destructive" role="alert"><AlertDescription>{{ deleteError }}</AlertDescription></Alert>
    <DialogFooter>
      <Button variant="outline" :disabled="deleting" @click="deleteTarget = null">{{ t('common.cancel') }}</Button>
      <Button variant="destructive" :disabled="deleting" @click="deletePersona">
        <Spinner v-if="deleting" data-icon="inline-start" aria-hidden="true" />{{ deleting ? t('common.deleting') : t('personas.deleteConfirm') }}
      </Button>
    </DialogFooter>
  </BusyDialog>
</template>
