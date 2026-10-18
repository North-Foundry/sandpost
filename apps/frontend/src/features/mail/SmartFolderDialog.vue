<script setup lang="ts">
import { useI18n } from 'vue-i18n'
import { computed, ref, watch } from 'vue'
import type { SavedView } from '@/api/contracts'
import { describeError } from '@/utils/errors'
import { Button } from '@/components/ui/button'
import { Checkbox } from '@/components/ui/checkbox'
import { DialogFooter } from '@/components/ui/dialog'
import { Field, FieldContent, FieldDescription, FieldError, FieldGroup, FieldLabel } from '@/components/ui/field'
import { Input } from '@/components/ui/input'
import { Spinner } from '@/components/ui/spinner'
import BusyDialog from '@/components/BusyDialog.vue'
import QueryEditor from '@/features/query/QueryEditor.vue'
import FolderIconPicker from './FolderIconPicker.vue'
import { folderIcon } from './folderIcons'
import { useWorkspace } from '@/features/shell/workspace'

const open = defineModel<boolean>('open', { required: true })
const { t } = useI18n()

/**
 * The smart folder being edited, or null to create one.
 */
const props = defineProps<{ view: SavedView | null; address?: string | null }>()
const emit = defineEmits<{ (event: 'saved', view: SavedView): void }>()
const workspace = useWorkspace()
const name = ref('')
const icon = ref<string | null>(null)
const filter = ref('')
const filterValid = ref<boolean | null>(null)
const shared = ref(false)
const saving = ref(false)
const error = ref('')

/**
 * Shared views are managed by instance administrators.
 */
const canShare = computed(() => workspace.canAdminister.value)
const editing = computed(() => props.view !== null)
const addressInbox = computed(() => Boolean(props.address && props.view))

/**
 * Save the canonical query as a personal or shared view.
 */
async function save(): Promise<void> {
  const user = workspace.user.value
  if (saving.value || !user || (!addressInbox.value && filterValid.value !== true)) return
  saving.value = true
  error.value = ''
  try {
    const saved = await workspace.saveView({
      id: props.view?.id ?? crypto.randomUUID(),
      ownerId: addressInbox.value ? props.view!.ownerId : shared.value && canShare.value ? null : user.id,
      name: name.value.trim(),
      filter: addressInbox.value ? props.view!.filter : filter.value.trim(),
      icon: icon.value,
    })
    open.value = false
    emit('saved', saved)
  } catch (failure) {
    error.value = describeError(failure, 'The smart folder could not be saved.')
  } finally { saving.value = false }
}

watch(open, (isOpen) => {
  if (!isOpen) return
  name.value = props.view?.name ?? ''
  icon.value = props.view?.icon ?? null
  filter.value = props.view?.filter ?? ''
  shared.value = props.view ? props.view.ownerId === null : false
  error.value = ''
})
</script>

<template>
  <BusyDialog
    v-model:open="open"
    :title="addressInbox ? t('mail.address.edit') : editing ? t('mail.smart.edit') : t('mail.smart.new')"
    :description="addressInbox ? t('mail.address.editDescription') : t('mail.smart.description')"
    :busy="saving"
    content-class="sm:max-w-xl"
  >
    <form class="flex flex-col gap-4" @submit.prevent="save">
      <FieldGroup class="gap-4">
        <Field :data-disabled="saving">
          <FieldLabel for="smart-folder-name">{{ t('mail.common.name') }}</FieldLabel>
          <Input id="smart-folder-name" v-model="name" :placeholder="t('mail.smart.namePlaceholder')" required maxlength="100" :disabled="saving" />
        </Field>
        <Field :data-disabled="saving">
          <FieldLabel for="smart-folder-icon">{{ t('mail.icon.label') }}</FieldLabel>
          <FolderIconPicker id="smart-folder-icon" v-model="icon" :default-icon="folderIcon(null, addressInbox ? 'address' : 'smart')" :disabled="saving" />
        </Field>
        <Field v-if="addressInbox">
          <FieldLabel for="saved-inbox-address">{{ t('mail.address.address') }}</FieldLabel>
          <Input id="saved-inbox-address" :model-value="props.address ?? ''" readonly />
        </Field>
        <Field v-else :data-disabled="saving" :data-invalid="filterValid === false">
          <FieldLabel for="smart-folder-query">{{ t('mail.smart.query') }}</FieldLabel>
          <QueryEditor
            id="smart-folder-query"
            v-model="filter"
            v-model:valid="filterValid"
            :disabled="saving"
            :empty-hint="t('mail.smart.emptyQuery')"
          />
        </Field>
        <Field v-if="canShare && !addressInbox" orientation="horizontal" :data-disabled="saving">
          <Checkbox id="smart-folder-shared" v-model="shared" :disabled="saving" />
          <FieldContent>
            <FieldLabel for="smart-folder-shared">{{ t('mail.smart.share') }}</FieldLabel>
            <FieldDescription>{{ t('mail.smart.shareDescription') }}</FieldDescription>
          </FieldContent>
        </Field>
        <FieldError v-if="error" role="alert">{{ error }}</FieldError>
      </FieldGroup>
      <DialogFooter>
        <Button variant="outline" type="button" :disabled="saving" @click="open = false">{{ t('mail.common.cancel') }}</Button>
        <Button type="submit" :disabled="saving || !name.trim() || (!addressInbox && filterValid !== true)">
          <Spinner v-if="saving" data-icon="inline-start" aria-hidden="true" />{{ saving ? t('mail.common.saving') : editing ? t('mail.smart.saveChanges') : t('mail.smart.create') }}
        </Button>
      </DialogFooter>
    </form>
  </BusyDialog>
</template>
