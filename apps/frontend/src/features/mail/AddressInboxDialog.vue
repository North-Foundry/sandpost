<script setup lang="ts">
import { useI18n } from 'vue-i18n'
import { ref, watch } from 'vue'
import type { Inbox } from '@/api/contracts'
import { Button } from '@/components/ui/button'
import { DialogFooter } from '@/components/ui/dialog'
import { Field, FieldDescription, FieldError, FieldGroup, FieldLabel } from '@/components/ui/field'
import { Input } from '@/components/ui/input'
import { Spinner } from '@/components/ui/spinner'
import BusyDialog from '@/components/BusyDialog.vue'
import { useWorkspace } from '@/features/shell/workspace'

const open = defineModel<boolean>('open', { required: true })
const { t } = useI18n()
const emit = defineEmits<{ (event: 'created', inbox: Inbox): void }>()
const workspace = useWorkspace()
const name = ref('')
const address = ref('')
const creating = ref(false)
const error = ref('')

/**
 * Save a recipient-filter inbox and hand it to the caller to open.
 */
async function create(): Promise<void> {
  if (creating.value) return
  creating.value = true
  error.value = ''
  try {
    const created = await workspace.createAddressInbox({ name: name.value.trim(), address: address.value.trim() })
    open.value = false
    emit('created', created)
  } catch (failure) {
    error.value = failure instanceof Error ? failure.message : t('mail.address.createError')
  } finally { creating.value = false }
}

watch(open, (isOpen) => {
  if (!isOpen) return
  name.value = ''
  address.value = ''
  error.value = ''
})
</script>

<template>
  <BusyDialog v-model:open="open" :title="t('mail.address.title')" :description="t('mail.address.description')" :busy="creating">
    <form class="flex flex-col gap-4" @submit.prevent="create">
      <FieldGroup class="gap-4">
        <Field :data-disabled="creating">
          <FieldLabel for="inbox-name">{{ t('mail.common.name') }}</FieldLabel>
          <Input id="inbox-name" v-model="name" :placeholder="t('mail.address.namePlaceholder')" required maxlength="100" :disabled="creating" />
        </Field>
        <Field :data-disabled="creating">
          <FieldLabel for="inbox-address">{{ t('mail.address.address') }}</FieldLabel>
          <Input id="inbox-address" v-model="address" type="email" :placeholder="t('mail.address.addressPlaceholder')" required maxlength="254" aria-describedby="inbox-address-hint" :disabled="creating" autocapitalize="none" :spellcheck="false" />
          <FieldDescription id="inbox-address-hint">{{ t('mail.address.hint') }}</FieldDescription>
        </Field>
        <FieldError v-if="error" role="alert">{{ error }}</FieldError>
      </FieldGroup>
      <DialogFooter>
        <Button variant="outline" type="button" :disabled="creating" @click="open = false">{{ t('mail.common.cancel') }}</Button>
        <Button type="submit" :disabled="creating || !name.trim() || !address.trim()">
          <Spinner v-if="creating" data-icon="inline-start" aria-hidden="true" />{{ creating ? t('mail.address.creating') : t('mail.address.create') }}
        </Button>
      </DialogFooter>
    </form>
  </BusyDialog>
</template>
