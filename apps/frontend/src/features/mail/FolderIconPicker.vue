<script setup lang="ts">
import type { Component } from 'vue'
import { computed, ref } from 'vue'
import { CheckIcon, ChevronDownIcon } from '@lucide/vue'
import { useI18n } from 'vue-i18n'
import { Button } from '@/components/ui/button'
import { Input } from '@/components/ui/input'
import { Popover, PopoverContent, PopoverTrigger } from '@/components/ui/popover'
import { FOLDER_ICON_OPTIONS } from './folderIcons'

const icon = defineModel<string | null>({ required: true })
const props = withDefaults(defineProps<{
  defaultIcon: Component
  disabled?: boolean
  id?: string
}>(), { disabled: false })
const { t } = useI18n()
const open = ref(false)
const search = ref('')
const currentIcon = computed(() => FOLDER_ICON_OPTIONS.find(option => option.id === icon.value)?.component ?? props.defaultIcon)
const filteredOptions = computed(() => {
  const query = search.value.trim().toLocaleLowerCase().replace(/[\s-]+/g, ' ')
  return query
    ? FOLDER_ICON_OPTIONS.filter(option => option.id.replaceAll('-', ' ').includes(query))
    : FOLDER_ICON_OPTIONS
})

/**
 * Convert a stable kebab-case icon ID into a readable option name.
 */
function optionLabel(id: string): string {
  return id.replaceAll('-', ' ')
}

/**
 * Select an explicit icon and close the picker.
 */
function selectIcon(id: string): void {
  icon.value = id
  open.value = false
}

/**
 * Clear the override so the owning folder kind supplies its default icon.
 */
function resetIcon(): void {
  icon.value = null
  open.value = false
}
</script>

<template>
  <div class="flex flex-col gap-2">
    <Popover v-model:open="open">
      <PopoverTrigger as-child>
        <Button
          :id="id"
          type="button"
          variant="outline"
          class="w-full justify-between"
          :disabled="disabled"
          :aria-label="t('mail.icon.label')"
        >
          <span class="flex items-center gap-2">
            <component :is="currentIcon" aria-hidden="true" />
            {{ icon ? optionLabel(icon) : t('mail.icon.default') }}
          </span>
          <ChevronDownIcon aria-hidden="true" />
        </Button>
      </PopoverTrigger>
      <PopoverContent align="start" class="flex w-72 flex-col gap-3">
        <Input
          v-model="search"
          type="search"
          :disabled="disabled"
          :aria-label="t('mail.icon.search')"
          :placeholder="t('mail.icon.search')"
        />
        <div class="grid max-h-64 grid-cols-5 gap-1 overflow-y-auto" role="group" :aria-label="t('mail.icon.choose')">
          <Button
            v-for="option in filteredOptions"
            :key="option.id"
            type="button"
            :variant="icon === option.id ? 'secondary' : 'ghost'"
            size="icon"
            :disabled="disabled"
            :aria-label="optionLabel(option.id)"
            :aria-pressed="icon === option.id"
            :title="optionLabel(option.id)"
            @click="selectIcon(option.id)"
          >
            <component :is="option.component" aria-hidden="true" />
          </Button>
          <p v-if="filteredOptions.length === 0" class="col-span-5 p-2" role="status">
            {{ t('mail.icon.empty') }}
          </p>
        </div>
        <Button type="button" variant="outline" class="w-full" :disabled="disabled" @click="resetIcon">
          <component :is="defaultIcon" data-icon="inline-start" aria-hidden="true" />
          {{ t('mail.icon.default') }}
          <CheckIcon v-if="icon === null" data-icon="inline-end" aria-hidden="true" />
        </Button>
      </PopoverContent>
    </Popover>
    <span v-if="icon" class="sr-only" aria-live="polite">{{ t('mail.icon.selected', { name: optionLabel(icon) }) }}</span>
  </div>
</template>
