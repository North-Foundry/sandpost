<script setup lang="ts">
import { useI18n } from 'vue-i18n'
import type { ImapLinkedView, SavedView } from '@/api/contracts'
import { Checkbox } from '@/components/ui/checkbox'
import { Field, FieldContent, FieldDescription, FieldLabel, FieldLegend, FieldSet } from '@/components/ui/field'
import { Input } from '@/components/ui/input'

/**
 * Choose which Smart Folders an IMAP account exposes, each with an optional IMAP mailbox name.
 * The selection keeps the order of `views`.
 */
const props = defineProps<{ views: SavedView[]; idPrefix: string; disabled?: boolean }>()
const links = defineModel<ImapLinkedView[]>('links', { required: true })
const { t } = useI18n()

/**
 * Return the selection entry for a Smart Folder, if it is selected.
 */
function linkFor(viewId: string): ImapLinkedView | undefined {
  return links.value.find((link) => link.viewId === viewId)
}

/**
 * Select or deselect a Smart Folder, keeping any alias typed earlier while it stays selected.
 */
function setSelected(viewId: string, selected: boolean): void {
  const kept = links.value.filter((link) => link.viewId !== viewId)
  const next = selected ? [...kept, linkFor(viewId) ?? { viewId, alias: null }] : kept
  const order = new Map(props.views.map((view, index) => [view.id, index]))
  links.value = next.sort((first, second) => (order.get(first.viewId) ?? 0) - (order.get(second.viewId) ?? 0))
}

/**
 * Change the IMAP mailbox name of a selected Smart Folder; blank follows the folder's name.
 */
function setAlias(viewId: string, alias: string): void {
  links.value = links.value.map((link) => link.viewId === viewId ? { ...link, alias: alias === '' ? null : alias } : link)
}
</script>

<template>
  <FieldSet class="gap-3">
    <FieldLegend variant="label">{{ t('imap.smartPicker.legend') }}</FieldLegend>
    <FieldDescription v-if="!views.length">{{ t('imap.smartPicker.empty') }}</FieldDescription>
    <div v-else class="flex max-h-72 flex-col gap-3 overflow-y-auto rounded-lg border p-3">
      <Field v-for="view in views" :key="view.id" orientation="horizontal" :data-disabled="disabled">
        <Checkbox
          :id="`${idPrefix}-view-${view.id}`"
          :model-value="Boolean(linkFor(view.id))"
          :disabled="disabled"
          @update:model-value="setSelected(view.id, $event === true)"
        />
        <FieldContent class="min-w-0">
          <FieldLabel :for="`${idPrefix}-view-${view.id}`" class="font-normal"><span class="truncate">{{ view.name }}</span></FieldLabel>
          <template v-if="linkFor(view.id)">
            <label :for="`${idPrefix}-alias-${view.id}`" class="sr-only">{{ t('imap.smartPicker.aliasLabel', { name: view.name }) }}</label>
            <Input
              :id="`${idPrefix}-alias-${view.id}`"
              :model-value="linkFor(view.id)?.alias ?? ''"
              :placeholder="t('imap.smartPicker.aliasPlaceholder', { name: view.name })"
              maxlength="100"
              autocomplete="off"
              class="h-7 text-xs"
              :disabled="disabled"
              @update:model-value="setAlias(view.id, String($event))"
            />
          </template>
        </FieldContent>
      </Field>
    </div>
  </FieldSet>
</template>
