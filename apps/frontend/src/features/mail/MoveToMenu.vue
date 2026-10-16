<script setup lang="ts">
import { useI18n } from 'vue-i18n'
import { CopyIcon, FolderInputIcon, Trash2Icon } from '@lucide/vue'
import { Button } from '@/components/ui/button'
import {
  DropdownMenu, DropdownMenuContent, DropdownMenuGroup, DropdownMenuItem, DropdownMenuSeparator, DropdownMenuTrigger,
} from '@/components/ui/dropdown-menu'
import type { MailFolder } from '@/features/shell/workspace'

/**
 * `trash` offers Trash as a destination too; it is left out inside Trash, where restoring is the only move.
 */
const props = defineProps<{ folders: MailFolder[]; disabled?: boolean; trash?: boolean; copy?: boolean }>()
const { t } = useI18n()
const emit = defineEmits<{
  /**
   * Requests moving the applicable messages to the destination.
   */
  (event: 'move', folderIdentifier: string | null): void

  /**
   * Requests copying the applicable messages to the destination.
   */
  (event: 'copy', folderIdentifier: string | null): void

  /**
   * Requests moving the applicable messages to Trash.
   */
  (event: 'trash'): void
}>()

/**
 * Lists manual folders as move and copy destinations.
 */
const destinations = () => props.folders.filter((folder) => folder.kind === 'folder')
</script>

<template>
  <DropdownMenu>
    <DropdownMenuTrigger as-child>
      <Button variant="outline" size="sm" :disabled="props.disabled">
        <CopyIcon v-if="props.copy" data-icon="inline-start" />
        <FolderInputIcon v-else data-icon="inline-start" />{{ props.copy ? t('mail.move.copyTo') : t('mail.move.moveTo') }}
      </Button>
    </DropdownMenuTrigger>
    <DropdownMenuContent align="end" class="w-56">
      <DropdownMenuGroup>
        <DropdownMenuItem @select="props.copy ? emit('copy', null) : emit('move', null)">{{ t('mail.move.inboxFolders') }}</DropdownMenuItem>
        <DropdownMenuItem v-for="folder in destinations()" :key="folder.id" @select="props.copy ? emit('copy', folder.id) : emit('move', folder.id)">
          {{ folder.name }}
        </DropdownMenuItem>
      </DropdownMenuGroup>
      <template v-if="props.trash">
        <DropdownMenuSeparator />
        <DropdownMenuGroup>
          <DropdownMenuItem @select="emit('trash')"><Trash2Icon />{{ t('mail.move.trash') }}</DropdownMenuItem>
        </DropdownMenuGroup>
      </template>
    </DropdownMenuContent>
  </DropdownMenu>
</template>
