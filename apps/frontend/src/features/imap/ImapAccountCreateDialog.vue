<script setup lang="ts">
import { ref, watch } from 'vue'
import { useI18n } from 'vue-i18n'
import type { ImapAccountMode, IssuedImapCredentials } from '@/api/contracts'
import { useSandpostClient } from '@/runtime/client'
import { describeError } from '@/utils/errors'
import { Button } from '@/components/ui/button'
import { DialogFooter } from '@/components/ui/dialog'
import { Field, FieldDescription, FieldError, FieldGroup, FieldLabel } from '@/components/ui/field'
import { Input } from '@/components/ui/input'
import { Spinner } from '@/components/ui/spinner'
import BusyDialog from '@/components/BusyDialog.vue'
import { ToggleGroup, ToggleGroupItem } from '@/components/ui/toggle-group'

/**
 * Create a personal IMAP account; the parent shows the issued password once.
 */
const open = defineModel<boolean>('open', { required: true })
const emit = defineEmits<{ (event: 'created', credentials: IssuedImapCredentials): void }>()
const client = useSandpostClient()
const { t } = useI18n()
const name = ref('')
const username = ref('')
const mode = ref<ImapAccountMode>('mirror')
const creating = ref(false)
const error = ref('')

/**
 * Create the account and hand its one-time credentials to the parent.
 */
async function create(): Promise<void> {
  if (creating.value || !name.value.trim()) return
  creating.value = true
  error.value = ''
  try {
    const credentials = await client.imap.create({
      name: name.value.trim(),
      username: username.value.trim() || null,
      mode: mode.value,
    })
    open.value = false
    emit('created', credentials)
  } catch (failure) {
    const message = describeError(failure, t('imap.errors.createAccount'))
    error.value = message === 'Conflict' ? t('imap.errors.usernameTaken') : message
  } finally {
    creating.value = false
  }
}

/**
 * Keep a mode selected when the active toggle is pressed again.
 */
function setMode(value: unknown): void {
  if (value === 'mirror' || value === 'independent') mode.value = value
}

watch(open, (isOpen) => {
  if (!isOpen) return
  name.value = ''
  username.value = ''
  mode.value = 'mirror'
  error.value = ''
})
</script>

<template>
  <BusyDialog
    v-model:open="open"
    :title="t('imap.create.title')"
    :description="t('imap.create.description')"
    :busy="creating"
    content-class="sm:max-w-lg"
  >
    <form class="flex flex-col gap-4" @submit.prevent="create">
      <FieldGroup class="gap-4">
        <Field :data-disabled="creating">
          <FieldLabel for="imap-account-name">{{ t('imap.common.name') }}</FieldLabel>
          <Input id="imap-account-name" v-model="name" :placeholder="t('imap.create.namePlaceholder')" required maxlength="100" autocomplete="off" :disabled="creating" />
          <FieldDescription>{{ t('imap.create.nameDescription') }}</FieldDescription>
        </Field>
        <Field :data-disabled="creating">
          <FieldLabel for="imap-account-username">{{ t('imap.connection.username') }} <span class="font-normal text-muted-foreground">({{ t('imap.common.optional') }})</span></FieldLabel>
          <Input id="imap-account-username" v-model="username" class="font-mono" maxlength="64" autocomplete="off" spellcheck="false" :disabled="creating" />
          <FieldDescription>{{ t('imap.create.usernameDescription') }}</FieldDescription>
        </Field>
        <Field>
          <FieldLabel>{{ t('imap.account.mode') }}</FieldLabel>
          <ToggleGroup :model-value="mode" @update:model-value="setMode" type="single" variant="outline" :disabled="creating" :aria-label="t('imap.account.mode')">
            <ToggleGroupItem value="mirror" :aria-label="t('imap.account.mirrorMode')">{{ t('imap.account.mirror') }}</ToggleGroupItem>
            <ToggleGroupItem value="independent" :aria-label="t('imap.account.independentMode')">{{ t('imap.account.independent') }}</ToggleGroupItem>
          </ToggleGroup>
          <FieldDescription>
            {{ t('imap.create.modeDescription') }}
          </FieldDescription>
        </Field>
        <FieldError v-if="error" role="alert">{{ error }}</FieldError>
      </FieldGroup>
      <DialogFooter>
        <Button variant="outline" type="button" :disabled="creating" @click="open = false">{{ t('imap.common.cancel') }}</Button>
        <Button type="submit" :disabled="creating || !name.trim()">
          <Spinner v-if="creating" data-icon="inline-start" aria-hidden="true" />{{ creating ? t('imap.create.creating') : t('imap.create.submit') }}
        </Button>
      </DialogFooter>
    </form>
  </BusyDialog>
</template>
