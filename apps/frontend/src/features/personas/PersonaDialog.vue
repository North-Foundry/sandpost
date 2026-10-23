<script setup lang="ts">
import { useI18n } from 'vue-i18n'
import { computed, ref, watch } from 'vue'
import { InfoIcon } from '@lucide/vue'
import type { Persona } from '@/api/contracts'
import { describeError } from '@/utils/errors'
import { Alert, AlertDescription } from '@/components/ui/alert'
import { Button } from '@/components/ui/button'
import { Checkbox } from '@/components/ui/checkbox'
import { DialogFooter } from '@/components/ui/dialog'
import { Field, FieldContent, FieldDescription, FieldError, FieldGroup, FieldLabel } from '@/components/ui/field'
import { Input } from '@/components/ui/input'
import { Spinner } from '@/components/ui/spinner'
import BusyDialog from '@/components/BusyDialog.vue'
import FolderIconPicker from '@/features/mail/FolderIconPicker.vue'
import { folderIcon } from '@/features/mail/folderIcons'
import { useWorkspace } from '@/features/shell/workspace'
import PersonaScriptEditor from './PersonaScriptEditor.vue'

const open = defineModel<boolean>('open', { required: true })

/**
 * The Persona being edited, or null to create one.
 */
const props = defineProps<{ persona: Persona | null }>()
const emit = defineEmits<{ (event: 'saved', persona: Persona): void }>()
const { t } = useI18n()
const workspace = useWorkspace()
const name = ref('')
const icon = ref<string | null>(null)
const address = ref('')
const script = ref('')
const scriptValid = ref<boolean | null>(null)
const enabled = ref(true)
const saving = ref(false)
const error = ref('')
const editing = computed(() => props.persona !== null)

/**
 * Explain an address whose access is not covered for every message.
 */
const accessNotice = computed(() => {
  const persona = props.persona
  if (!persona || persona.address !== address.value.trim().toLowerCase()) return ''
  if (persona.addressAccess === 'perMessage') return t('personas.access.perMessage')
  if (persona.addressAccess === 'denied') return t('personas.access.denied')
  return ''
})

/**
 * Create or update the Persona, keeping the dialog open with the server's explanation on failure.
 */
async function save(): Promise<void> {
  if (saving.value || scriptValid.value !== true) return
  saving.value = true
  error.value = ''
  try {
    const saved = await workspace.savePersona(props.persona?.id ?? null, {
      name: name.value.trim(),
      icon: icon.value,
      address: address.value.trim(),
      script: script.value,
      enabled: enabled.value,
    })
    open.value = false
    emit('saved', saved)
  } catch (failure) {
    error.value = describeError(failure, t('personas.saveFailed'))
  } finally { saving.value = false }
}

watch(open, (isOpen) => {
  if (!isOpen) return
  name.value = props.persona?.name ?? ''
  icon.value = props.persona?.icon ?? null
  address.value = props.persona?.address ?? ''
  script.value = props.persona?.script ?? 'wait 5s;\nopen;\n'
  enabled.value = props.persona?.enabled ?? true
  error.value = ''
}, { immediate: true })
</script>

<template>
  <BusyDialog
    v-model:open="open"
    :title="editing ? t('personas.dialog.editTitle') : t('personas.dialog.createTitle')"
    :description="t('personas.dialog.description')"
    :busy="saving"
    content-class="sm:max-w-2xl"
  >
    <form class="flex flex-col gap-4" @submit.prevent="save">
      <FieldGroup class="gap-4">
        <div class="grid gap-4 sm:grid-cols-2">
          <Field :data-disabled="saving">
            <FieldLabel for="persona-name">{{ t('personas.field.name') }}</FieldLabel>
            <Input id="persona-name" v-model="name" :placeholder="t('personas.field.namePlaceholder')" required maxlength="100" autocomplete="off" :disabled="saving" />
          </Field>
          <Field :data-disabled="saving">
            <FieldLabel for="persona-address">{{ t('personas.field.address') }}</FieldLabel>
            <Input id="persona-address" v-model="address" type="email" :placeholder="t('personas.field.addressPlaceholder')" required maxlength="254" autocomplete="off" spellcheck="false" class="font-mono" :disabled="saving" aria-describedby="persona-address-hint" />
          </Field>
        </div>
        <FieldDescription id="persona-address-hint" class="-mt-2">{{ t('personas.field.addressHint') }}</FieldDescription>
        <Field :data-disabled="saving">
          <FieldLabel for="persona-icon">{{ t('mail.icon.label') }}</FieldLabel>
          <FolderIconPicker id="persona-icon" v-model="icon" :default-icon="folderIcon(null, 'persona')" :disabled="saving" />
        </Field>
        <Alert v-if="accessNotice">
          <InfoIcon />
          <AlertDescription>{{ accessNotice }}</AlertDescription>
        </Alert>
        <Field :data-disabled="saving" :data-invalid="scriptValid === false">
          <FieldLabel for="persona-script">{{ t('personas.field.script') }}</FieldLabel>
          <PersonaScriptEditor id="persona-script" v-model="script" v-model:valid="scriptValid" :disabled="saving" />
        </Field>
        <Field orientation="horizontal" :data-disabled="saving">
          <Checkbox id="persona-enabled" v-model="enabled" :disabled="saving" />
          <FieldContent>
            <FieldLabel for="persona-enabled">{{ t('personas.field.enabled') }}</FieldLabel>
            <FieldDescription>{{ t('personas.field.enabledHint') }}</FieldDescription>
          </FieldContent>
        </Field>
        <FieldError v-if="error" role="alert">{{ error }}</FieldError>
      </FieldGroup>
      <DialogFooter>
        <Button variant="outline" type="button" :disabled="saving" @click="open = false">{{ t('common.cancel') }}</Button>
        <Button type="submit" :disabled="saving || !name.trim() || !address.trim() || scriptValid !== true">
          <Spinner v-if="saving" data-icon="inline-start" aria-hidden="true" />{{ saving ? t('personas.saving') : editing ? t('personas.save') : t('personas.create') }}
        </Button>
      </DialogFooter>
    </form>
  </BusyDialog>
</template>
