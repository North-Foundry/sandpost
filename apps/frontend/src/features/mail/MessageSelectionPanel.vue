<script setup lang="ts">
import { useI18n } from 'vue-i18n'
import { MailsIcon, Trash2Icon } from '@lucide/vue'
import type { MessageFlags, MessageFlagOperation } from '@/api/contracts'
import { Button } from '@/components/ui/button'
import { Empty, EmptyContent, EmptyDescription, EmptyHeader, EmptyMedia, EmptyTitle } from '@/components/ui/empty'
import { Kbd } from '@/components/ui/kbd'
import type { MailFolder } from '@/features/shell/workspace'
import MoveToMenu from './MoveToMenu.vue'
import MessageFlagsMenu from './MessageFlagsMenu.vue'

/**
 * Inside `trash`, Delete removes messages permanently; elsewhere it moves them to Trash.
 */
const props = withDefaults(defineProps<{ count: number; deleting: boolean; folders?: MailFolder[]; flags?: MessageFlags[]; trash?: boolean }>(), {
  /**
   * Give each selection panel its own empty destination list by default.
   */
  folders: () => [],

  /**
   * Give each selection panel its own empty collection of selected message flags.
   */
  flags: () => [], trash: false,
})
const emit = defineEmits<{
  /**
   * Requests deleting the applicable message or messages.
   */
  (event: 'delete'): void

  /**
   * Requests moving the applicable messages to the destination.
   */
  (event: 'move', folderIdentifier: string | null): void

  /**
   * Requests copying the applicable messages to the destination.
   */
  (event: 'copy', folderIdentifier: string | null): void

  /**
   * Requests applying the specified flag operation.
   */
  (event: 'set-flags', operation: MessageFlagOperation, flags: MessageFlags): void

  /**
   * Requests clearing the bulk selection.
   */
  (event: 'clear'): void
}>()

/**
 * The modifier for toggling one message: Command on macOS, Control elsewhere.
 */
const toggleModifier = typeof navigator !== 'undefined' && /Mac|iPhone|iPad/.test(navigator.platform) ? '⌘' : 'Ctrl'
const { t } = useI18n()
</script>

<template>
  <section class="flex min-h-0 min-w-0 flex-col bg-background" :aria-label="t('mail.selection.label')">
    <Empty>
      <EmptyHeader>
        <EmptyMedia variant="icon"><MailsIcon /></EmptyMedia>
        <EmptyTitle class="tabular-nums">{{ t('mail.selection.count', { count: props.count }) }}</EmptyTitle>
        <EmptyDescription>
          {{ t('mail.selection.rangeHelp', { modifier: toggleModifier }) }}
          <Kbd>Delete</Kbd> {{ props.trash ? t('mail.selection.deleteHelp') : t('mail.selection.trashHelp') }}
        </EmptyDescription>
      </EmptyHeader>
      <EmptyContent class="flex-row flex-wrap justify-center">
        <Button variant="destructive" size="sm" :disabled="props.deleting" @click="emit('delete')">
          <Trash2Icon data-icon="inline-start" />{{ props.trash ? t('mail.selection.deleteCount', { count: props.count }) : t('mail.selection.trashCount', { count: props.count }) }}
        </Button>
        <MoveToMenu :folders="props.folders" :disabled="props.deleting" @move="emit('move', $event)" />
        <MoveToMenu :folders="props.folders" :disabled="props.deleting" copy @copy="emit('copy', $event)" />
        <MessageFlagsMenu :flags="props.flags" :disabled="props.deleting" @change="(operation, flags) => emit('set-flags', operation, flags)" />
        <Button variant="outline" size="sm" :disabled="props.deleting" @click="emit('clear')">{{ t('mail.list.clearSelection') }}</Button>
      </EmptyContent>
    </Empty>
  </section>
</template>
