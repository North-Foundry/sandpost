<script setup lang="ts">
import { useI18n } from 'vue-i18n'
import { computed, ref, watch } from 'vue'
import { FlagIcon, MessageCircleReplyIcon, PencilIcon } from '@lucide/vue'
import type { MessageFlags, MessageFlagOperation } from '@/api/contracts'
import { Button } from '@/components/ui/button'
import { DialogFooter } from '@/components/ui/dialog'
import { Field, FieldDescription, FieldGroup, FieldLabel } from '@/components/ui/field'
import { Textarea } from '@/components/ui/textarea'
import { DropdownMenu, DropdownMenuCheckboxItem, DropdownMenuContent, DropdownMenuGroup, DropdownMenuItem, DropdownMenuTrigger } from '@/components/ui/dropdown-menu'
import BusyDialog from '@/components/BusyDialog.vue'

const props = withDefaults(defineProps<{ flags: MessageFlags[]; disabled?: boolean }>(), { disabled: false })
const { t } = useI18n()
const emit = defineEmits<{ (event: 'change', operation: MessageFlagOperation, flags: MessageFlags): void }>()
const editingKeywords = ref(false)
const keywords = ref('')
const current = computed(() => props.flags[0] ?? { seen: false, answered: false, flagged: false, draft: false, deleted: false, keywords: [] })
const multiple = computed(() => props.flags.length > 1)
const keywordLabel = computed(() => multiple.value ? t('mail.flags.multipleHint') : t('mail.flags.singleHint'))

/**
 * Reports whether every displayed message has the requested flag.
 */
function checked(key: 'answered' | 'flagged' | 'draft'): boolean {
  return props.flags.length > 0 && props.flags.every((flags) => flags[key])
}

/**
 * Emits an add or remove operation for one standard IMAP flag.
 */
function toggle(key: 'answered' | 'flagged' | 'draft'): void {
  const remove = checked(key)
  const flags: MessageFlags = { seen: false, answered: false, flagged: false, draft: false, deleted: false, keywords: [] }
  flags[key] = true
  emit('change', remove ? 'remove' : 'add', flags)
}

watch(editingKeywords, (open) => {
  if (open) keywords.value = current.value.keywords.join(' ')
})

/**
 * Replaces keywords after trimming and case-insensitively deduplicating them.
 */
function saveKeywords(): void {
  const seen = new Set<string>()
  const nextKeywords = keywords.value.split(/[\s,]+/).map((word) => word.trim()).filter((word) => {
    const key = word.toLowerCase()
    if (!word || seen.has(key)) return false
    seen.add(key)
    return true
  })
  emit('change', 'replace', { ...current.value, deleted: false, keywords: nextKeywords })
  editingKeywords.value = false
}
</script>

<template>
  <DropdownMenu>
    <DropdownMenuTrigger as-child>
      <Button variant="outline" size="sm" :disabled="props.disabled" :aria-label="t('mail.flags.label')">{{ t('mail.flags.title') }}</Button>
    </DropdownMenuTrigger>
    <DropdownMenuContent align="end" class="w-56">
      <DropdownMenuGroup>
        <DropdownMenuCheckboxItem :model-value="checked('flagged')" @select.prevent="toggle('flagged')"><FlagIcon />{{ t('mail.flags.flagged') }}</DropdownMenuCheckboxItem>
        <DropdownMenuCheckboxItem :model-value="checked('answered')" @select.prevent="toggle('answered')"><MessageCircleReplyIcon />{{ t('mail.flags.answered') }}</DropdownMenuCheckboxItem>
        <DropdownMenuCheckboxItem :model-value="checked('draft')" @select.prevent="toggle('draft')">{{ t('mail.flags.draft') }}</DropdownMenuCheckboxItem>
        <DropdownMenuItem :disabled="multiple || props.disabled" @select="editingKeywords = true">
          <PencilIcon />{{ t('mail.flags.keywords') }}: {{ current.keywords.length ? current.keywords.join(' ') : '—' }}
        </DropdownMenuItem>
      </DropdownMenuGroup>
    </DropdownMenuContent>
  </DropdownMenu>

  <BusyDialog v-model:open="editingKeywords" :title="t('mail.flags.editKeywords')" :description="t('mail.flags.description')" :busy="props.disabled">
    <FieldGroup>
      <Field :data-disabled="props.disabled">
        <FieldLabel for="message-keywords">{{ t('mail.flags.keywords') }}</FieldLabel>
        <Textarea id="message-keywords" v-model="keywords" rows="3" :disabled="props.disabled" aria-describedby="message-keywords-hint" />
        <FieldDescription id="message-keywords-hint">{{ keywordLabel }}</FieldDescription>
      </Field>
    </FieldGroup>
    <DialogFooter>
      <Button variant="outline" :disabled="props.disabled" @click="editingKeywords = false">{{ t('mail.common.cancel') }}</Button>
      <Button :disabled="props.disabled" @click="saveKeywords">{{ t('mail.flags.saveKeywords') }}</Button>
    </DialogFooter>
  </BusyDialog>
</template>
