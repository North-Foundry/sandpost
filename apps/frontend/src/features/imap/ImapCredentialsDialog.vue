<script setup lang="ts">
import { computed, ref, watch } from 'vue'
import { useI18n } from 'vue-i18n'
import { CheckIcon, CopyIcon, TriangleAlertIcon } from '@lucide/vue'
import type { IssuedImapCredentials } from '@/api/contracts'
import { Alert, AlertDescription, AlertTitle } from '@/components/ui/alert'
import { Button } from '@/components/ui/button'
import { Checkbox } from '@/components/ui/checkbox'
import { Dialog, DialogContent, DialogDescription, DialogFooter, DialogHeader, DialogTitle } from '@/components/ui/dialog'
import { Field, FieldError, FieldGroup, FieldLabel } from '@/components/ui/field'
import { InputGroup, InputGroupAddon, InputGroupButton, InputGroupInput } from '@/components/ui/input-group'
import { Separator } from '@/components/ui/separator'
import ImapServerNotice from './ImapServerNotice.vue'
import ImapSetupInstructions from './ImapSetupInstructions.vue'
import { imapConnectionFields, type ImapConnectionField } from './security'
import { useCopy } from '@/composables/useCopy'

/**
 * Show freshly issued IMAP credentials exactly once.
 *
 * The parent owns the credentials and must drop them on `closed`; this dialog keeps no copy of the
 * password and can only be dismissed after the user confirms the credentials were saved.
 */
const props = defineProps<{ credentials: IssuedImapCredentials | null; regenerated: boolean }>()
const emit = defineEmits<{
  /**
   * Tell the parent to discard the one-time credentials.
   */
  (event: 'closed'): void
}>()

type CopyTarget = ImapConnectionField['key'] | 'all'

const acknowledged = ref(false)
const { t, locale } = useI18n()
const { copied, error: copyError, copy: copyValue, reset: resetCopy } = useCopy<CopyTarget>()

const fields = computed(() => {
  locale.value
  return props.credentials
    ? imapConnectionFields(props.credentials.server, props.credentials.account, props.credentials.password)
    : []
})

/**
 * Copy one value, or every connection detail, and briefly confirm it inline.
 */
async function copy(target: CopyTarget): Promise<void> {
  const text = target === 'all'
    ? fields.value.map((field) => `${field.label}: ${field.value}`).join('\n')
    : fields.value.find((field) => field.key === target)?.value ?? ''
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
      class="max-h-[calc(100dvh-2rem)] overflow-y-auto sm:max-w-xl"
      :show-close-button="acknowledged"
      @escape-key-down="!acknowledged && $event.preventDefault()"
      @pointer-down-outside="$event.preventDefault()"
      @interact-outside="$event.preventDefault()"
    >
      <DialogHeader>
        <DialogTitle>{{ regenerated ? t('imap.credentials.newPassword') : t('imap.credentials.created') }}</DialogTitle>
        <DialogDescription>{{ t('imap.credentials.detailsFor') }} <span class="font-medium text-foreground">{{ credentials?.account.name }}</span>.</DialogDescription>
      </DialogHeader>

      <Alert class="border-warning/40">
        <TriangleAlertIcon class="text-warning!" />
        <AlertTitle>{{ t('imap.credentials.savePassword') }}</AlertTitle>
        <AlertDescription>{{ t('imap.credentials.passwordShownOnce') }}</AlertDescription>
      </Alert>
      <Alert v-if="regenerated" variant="destructive">
        <AlertTitle>{{ t('imap.credentials.previousPasswordInvalid') }}</AlertTitle>
        <AlertDescription>{{ t('imap.credentials.previousPasswordDescription') }}</AlertDescription>
      </Alert>
      <ImapServerNotice v-if="credentials" :server="credentials.server" />

      <FieldGroup class="gap-3">
        <Field v-for="field in fields" :key="field.key">
          <FieldLabel :for="`imap-credential-${field.key}`">{{ field.label }}</FieldLabel>
          <InputGroup>
            <InputGroupInput
              :id="`imap-credential-${field.key}`"
              :model-value="field.value"
              readonly
              class="font-mono"
              autocomplete="off"
              spellcheck="false"
              @focus="($event.target as HTMLInputElement).select()"
            />
            <InputGroupAddon align="inline-end">
              <InputGroupButton size="icon-xs" :aria-label="t('imap.credentials.copyField', { field: field.label.toLowerCase() })" @click="copy(field.key)">
                <CheckIcon v-if="copied === field.key" class="text-success" />
                <CopyIcon v-else />
              </InputGroupButton>
            </InputGroupAddon>
          </InputGroup>
        </Field>
        <FieldError v-if="copyError" role="alert">{{ copyError }}</FieldError>
      </FieldGroup>

      <Separator />
      <div class="flex flex-col gap-3">
        <h3 class="text-sm font-medium">{{ t('imap.credentials.setupClient') }}</h3>
        <ImapSetupInstructions v-if="credentials" :server="credentials.server" :username="credentials.account.username" />
      </div>

      <Field orientation="horizontal">
        <Checkbox id="imap-credentials-saved" v-model="acknowledged" />
        <FieldLabel for="imap-credentials-saved" class="font-normal">{{ t('imap.credentials.savedConfirmation') }}</FieldLabel>
      </Field>

      <DialogFooter class="sm:justify-between">
        <Button variant="outline" type="button" @click="copy('all')">
          <CheckIcon v-if="copied === 'all'" data-icon="inline-start" class="text-success" />
          <CopyIcon v-else data-icon="inline-start" />
          {{ t('imap.credentials.copyDetails') }}
        </Button>
        <Button type="button" :disabled="!acknowledged" @click="close">{{ t('imap.common.done') }}</Button>
      </DialogFooter>
    </DialogContent>
  </Dialog>
</template>
