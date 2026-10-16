<script setup lang="ts">
import { computed, ref, watch } from 'vue'
import { useI18n } from 'vue-i18n'
import type { ImapLocalFolder } from '@/api/contracts'
import { useSandpostClient } from '@/runtime/client'
import { describeError } from '@/utils/errors'
import { Button } from '@/components/ui/button'
import { DialogFooter } from '@/components/ui/dialog'
import { Field, FieldError, FieldGroup, FieldLabel } from '@/components/ui/field'
import { Input } from '@/components/ui/input'
import { Spinner } from '@/components/ui/spinner'
import BusyDialog from '@/components/BusyDialog.vue'
import QueryEditor from '@/features/query/QueryEditor.vue'

/**
 * Create or edit a Smart Folder owned by one Independent account.
 */
const open = defineModel<boolean>('open', { required: true })
const props = defineProps<{
  accountId: string

  /**
   * The folder being edited, or null to create one.
   */
  folder: ImapLocalFolder | null
}>()
const emit = defineEmits<{
  /**
   * Return the saved folder definition to its owning account page.
   */
  (event: 'saved', folder: ImapLocalFolder): void
}>()
const client = useSandpostClient()
const { t } = useI18n()
const name = ref('')
const filter = ref('')
const filterValid = ref<boolean | null>(null)
const saving = ref(false)
const error = ref('')

const editing = computed(() => props.folder !== null)

/**
 * Save the folder and report it to the parent.
 */
async function save(): Promise<void> {
  if (saving.value || !name.value.trim() || filterValid.value !== true) return
  saving.value = true
  error.value = ''
  const input = { name: name.value.trim(), filter: filter.value.trim() }
  try {
    const saved = props.folder
      ? await client.imap.folders.update(props.accountId, props.folder.id, input)
      : await client.imap.folders.create(props.accountId, input)
    open.value = false
    emit('saved', saved)
  } catch (failure) {
    const message = describeError(failure, t('imap.errors.saveFolder'))
    error.value = message === 'Conflict' ? t('imap.errors.folderNameTaken') : message
  } finally { saving.value = false }
}

watch(open, (isOpen) => {
  if (!isOpen) return
  name.value = props.folder?.name ?? ''
  filter.value = props.folder?.filter ?? ''
  error.value = ''
})
</script>

<template>
  <BusyDialog
    v-model:open="open"
    :title="editing ? t('imap.folder.editTitle') : t('imap.folder.newTitle')"
    :description="t('imap.folder.description')"
    :busy="saving"
    content-class="sm:max-w-xl"
  >
    <form class="flex flex-col gap-4" @submit.prevent="save">
      <FieldGroup class="gap-4">
        <Field :data-disabled="saving">
          <FieldLabel for="imap-folder-name">{{ t('imap.folder.mailboxName') }}</FieldLabel>
          <Input id="imap-folder-name" v-model="name" :placeholder="t('imap.folder.namePlaceholder')" required maxlength="100" autocomplete="off" :disabled="saving" />
        </Field>
        <Field :data-disabled="saving" :data-invalid="filterValid === false">
          <FieldLabel for="imap-folder-query">{{ t('imap.folder.query') }}</FieldLabel>
          <QueryEditor
            id="imap-folder-query"
            v-model="filter"
            v-model:valid="filterValid"
            :disabled="saving"
            :empty-hint="t('imap.folder.emptyQueryHint')"
          />
        </Field>
        <FieldError v-if="error" role="alert">{{ error }}</FieldError>
      </FieldGroup>
      <DialogFooter>
        <Button variant="outline" type="button" :disabled="saving" @click="open = false">{{ t('imap.common.cancel') }}</Button>
        <Button type="submit" :disabled="saving || !name.trim() || filterValid !== true">
          <Spinner v-if="saving" data-icon="inline-start" aria-hidden="true" />{{ saving ? t('imap.common.saving') : editing ? t('imap.common.saveChanges') : t('imap.folder.create') }}
        </Button>
      </DialogFooter>
    </form>
  </BusyDialog>
</template>
