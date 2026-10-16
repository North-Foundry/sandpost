<script setup lang="ts">
import { computed, ref, watch } from 'vue'
import { useI18n } from 'vue-i18n'
import { CheckIcon, CopyIcon, TriangleAlertIcon } from '@lucide/vue'
import type { IssuedSmtpCredentials } from '@/api/contracts'
import { credentialProtection } from '@/features/admin/encryption'
import { Alert, AlertDescription, AlertTitle } from '@/components/ui/alert'
import { Button } from '@/components/ui/button'
import { Checkbox } from '@/components/ui/checkbox'
import { Dialog, DialogContent, DialogDescription, DialogFooter, DialogHeader, DialogTitle } from '@/components/ui/dialog'
import { Field, FieldDescription, FieldError, FieldGroup, FieldLabel } from '@/components/ui/field'
import { InputGroup, InputGroupAddon, InputGroupButton, InputGroupInput } from '@/components/ui/input-group'
import { connectionAddress } from './listenAddress'
import { useCopy } from '@/composables/useCopy'

/**
 * Show freshly issued SMTP credentials exactly once.
 *
 * The parent owns the credentials and must drop them on `closed`; this dialog keeps no copy of the
 * password and can only be dismissed after the administrator confirms the credentials were saved.
 */
const props = defineProps<{ credentials: IssuedSmtpCredentials | null; regenerated: boolean }>()
const { t } = useI18n()
const emit = defineEmits<{
  /**
   * Tell the parent to discard the one-time credentials.
   */
  (event: 'closed'): void
}>()

type CopyTarget = 'host' | 'port' | 'username' | 'password' | 'all'

const acknowledged = ref(false)
const { copied, error: copyError, copy: copyValue, reset: resetCopy } = useCopy<CopyTarget>()

const connection = computed(() => {
  const server = props.credentials?.server
  const address = server?.listener?.boundAddress ?? server?.listener?.configuredAddress ?? server?.listenAddress
  return address ? connectionAddress(address) : null
})
const fields = computed(() => {
  const credentials = props.credentials
  if (!credentials) return []
  return [
    { target: 'host' as const, label: t('admin.smtp.host'), value: connection.value?.host ?? t('admin.smtp.serverDisabled') },
    { target: 'port' as const, label: t('admin.smtp.port'), value: connection.value?.port ?? '—' },
    { target: 'username' as const, label: t('admin.smtp.username'), value: credentials.access.username },
    { target: 'password' as const, label: t('admin.smtp.password'), value: credentials.password },
  ]
})
const authenticationNote = computed(() => {
  const credentials = props.credentials
  const listener = credentials?.server.listener
  const mechanisms = (credentials?.access.allowedMechanisms ?? ['plain', 'login']).map((mechanism) => mechanism.toUpperCase()).join(' or ')
  if (credentials?.access.requiresEncryption) {
    return t('admin.smtp.authWithTls', { mechanisms })
  }
  if (listener?.authenticationRequired === false) return t('admin.smtp.authLocalOptional')
  return t('admin.smtp.authWithProtection', { mechanisms, protection: credentialProtection(listener?.encryption ?? 'none') })
})

/**
 * Copy one value, or every connection detail, and briefly confirm it inline.
 */
async function copy(target: CopyTarget): Promise<void> {
  const text = target === 'all'
    ? fields.value.map((field) => `${field.label}: ${field.value}`).join('\n')
    : fields.value.find((field) => field.target === target)?.value ?? ''
  await copyValue(text, target)
}

/**
 * Close only after the explicit confirmation; the parent then forgets the password.
 */
function close(): void {
  if (!acknowledged.value) return
  emit('closed')
}

watch(() => props.credentials, () => {
  acknowledged.value = false
  resetCopy()
})
</script>

<template>
  <Dialog :open="Boolean(credentials)" @update:open="!$event && close()">
    <DialogContent
      class="max-h-[calc(100dvh-2rem)] overflow-y-auto sm:max-w-lg"
      :show-close-button="acknowledged"
      @escape-key-down="!acknowledged && $event.preventDefault()"
      @pointer-down-outside="$event.preventDefault()"
      @interact-outside="$event.preventDefault()"
    >
      <DialogHeader>
        <DialogTitle>{{ regenerated ? t('admin.smtp.newPassword') : t('admin.smtp.accessCreated') }}</DialogTitle>
        <DialogDescription>{{ t('admin.smtp.connectionDetails', { name: credentials?.access.name }) }}</DialogDescription>
      </DialogHeader>

      <Alert class="border-warning/40">
        <TriangleAlertIcon class="text-warning!" />
        <AlertTitle>{{ t('admin.smtp.savePasswordNow') }}</AlertTitle>
        <AlertDescription>{{ t('admin.smtp.passwordShownOnce') }}</AlertDescription>
      </Alert>
      <Alert v-if="regenerated" variant="destructive">
          <AlertTitle>{{ t('admin.smtp.previousPasswordInvalid') }}</AlertTitle>
          <AlertDescription>{{ t('admin.smtp.updateApplications') }}</AlertDescription>
      </Alert>

      <FieldGroup class="gap-3">
        <Field v-for="field in fields" :key="field.target">
          <FieldLabel :for="`smtp-credential-${field.target}`">{{ field.label }}</FieldLabel>
          <InputGroup>
            <InputGroupInput
              :id="`smtp-credential-${field.target}`"
              :model-value="field.value"
              readonly
              class="font-mono"
              autocomplete="off"
              spellcheck="false"
              @focus="($event.target as HTMLInputElement).select()"
            />
            <InputGroupAddon align="inline-end">
              <InputGroupButton size="icon-xs" :aria-label="t('admin.smtp.copyField', { field: field.label })" @click="copy(field.target)">
                <CheckIcon v-if="copied === field.target" class="text-success" />
                <CopyIcon v-else />
              </InputGroupButton>
            </InputGroupAddon>
          </InputGroup>
        </Field>
        <FieldDescription>{{ authenticationNote }}</FieldDescription>
        <FieldError v-if="copyError" role="alert">{{ copyError }}</FieldError>
      </FieldGroup>

      <Field orientation="horizontal">
        <Checkbox id="smtp-credentials-saved" v-model="acknowledged" />
        <FieldLabel for="smtp-credentials-saved" class="font-normal">{{ t('admin.smtp.savedCredentials') }}</FieldLabel>
      </Field>

      <DialogFooter class="sm:justify-between">
        <Button variant="outline" type="button" @click="copy('all')">
          <CheckIcon v-if="copied === 'all'" data-icon="inline-start" class="text-success" />
          <CopyIcon v-else data-icon="inline-start" />
          {{ t('admin.smtp.copyDetails') }}
        </Button>
        <Button type="button" :disabled="!acknowledged" @click="close">{{ t('admin.smtp.done') }}</Button>
      </DialogFooter>
    </DialogContent>
  </Dialog>
</template>
