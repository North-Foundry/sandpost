<script setup lang="ts">
import { useI18n } from 'vue-i18n'
import { ref, watch } from 'vue'
import type { WebFolder, WebFolderInput } from '@/api/contracts'
import { Alert, AlertDescription } from '@/components/ui/alert'
import { Button } from '@/components/ui/button'
import { DialogFooter } from '@/components/ui/dialog'
import { Field, FieldGroup, FieldLabel } from '@/components/ui/field'
import { Input } from '@/components/ui/input'
import { Spinner } from '@/components/ui/spinner'
import BusyDialog from '@/components/BusyDialog.vue'
import { useWorkspace } from '@/features/shell/workspace'

const open = defineModel<boolean>('open', { required: true })
const { t } = useI18n()
const props = defineProps<{ folder: WebFolder | null }>()
const emit = defineEmits<{ (event: 'saved', folder: WebFolder): void }>()
const workspace = useWorkspace()
const name = ref('')
const saving = ref(false)
const error = ref('')

/**
 * Saves the manual folder and emits the persisted folder after closing the dialog.
 */
async function save(): Promise<void> {
  if (saving.value) return
  saving.value = true
  error.value = ''
  try {
    const input: WebFolderInput = { name: name.value.trim() }
    const saved = await workspace.saveManualFolder(props.folder?.id ?? null, input)
    open.value = false
    emit('saved', saved)
  } catch (failure) {
    error.value = failure instanceof Error ? failure.message : t('mail.folder.saveError')
  } finally { saving.value = false }
}

watch(open, (isOpen) => {
  if (!isOpen) return
  name.value = props.folder?.name ?? ''
  error.value = ''
})
</script>

<template>
  <BusyDialog
    v-model:open="open"
    :title="props.folder ? t('mail.folder.edit') : t('mail.folder.new')"
    :description="t('mail.folder.description')"
    :busy="saving"
  >
    <form class="flex flex-col gap-4" @submit.prevent="save">
      <FieldGroup class="gap-4">
        <Field :data-disabled="saving">
          <FieldLabel for="web-folder-name">{{ t('mail.common.name') }}</FieldLabel>
          <Input id="web-folder-name" v-model="name" :placeholder="t('mail.folder.namePlaceholder')" required maxlength="100" :disabled="saving" />
        </Field>
        <Alert v-if="error" variant="destructive" role="alert"><AlertDescription>{{ error }}</AlertDescription></Alert>
      </FieldGroup>
      <DialogFooter>
        <Button variant="outline" type="button" :disabled="saving" @click="open = false">{{ t('mail.common.cancel') }}</Button>
        <Button type="submit" :disabled="saving || !name.trim()">
          <Spinner v-if="saving" data-icon="inline-start" aria-hidden="true" />{{ saving ? t('mail.common.saving') : props.folder ? t('mail.folder.save') : t('mail.folder.create') }}
        </Button>
      </DialogFooter>
    </form>
  </BusyDialog>
</template>
